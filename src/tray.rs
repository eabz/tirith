//! `tirith tray`: a macOS menu bar icon listing every Tirith daemon on
//! this machine, read from the per-user registry (`registry.rs`). Clicking
//! the icon always shows the menu: one row per daemon with its agent and
//! claim counts, and how many escalations wait for the human (the daemon's
//! human queue, ADR-0027). Clicking a row opens that daemon's dashboard,
//! `Stop` sends it SIGINT, `Quit tray` exits. See ADR-0019. When a
//! daemon's human queue grows, a macOS notification says so (through
//! `osascript`, so no notification entitlement or app bundle is needed).
//!
//! `AppKit` needs its event loop on the main thread. Rather than pull in
//! `winit`, this module pumps the loop by hand: wait for an event or a
//! five-second timeout, dispatch it, drain the menu and tray channels,
//! refresh the registry. Everything is the safe surface of `objc2`.

use std::collections::{BTreeSet, HashMap};
use std::process::Command;
use std::time::{Duration, Instant};

use muda::{IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEventMask};
use objc2_foundation::{MainThreadMarker, NSDate, NSString};
use serde_json::Value;
use thiserror::Error;
use tray_icon::{Icon, TrayIconBuilder, TrayIconEvent};

use crate::registry::{DaemonEntry, Registry, RegistryError};

/// How often the registry and each daemon's counts are re-read.
pub const POLL: Duration = Duration::from_secs(5);

/// How long one daemon gets to answer `/api/state` before it counts as
/// dead for this poll.
const DAEMON_TIMEOUT: Duration = Duration::from_millis(800);

/// The template icon: a tower with battlements and a flag, 22 by 22
/// points, `#` opaque black, `.` transparent. Rasterized at 2x at start.
const TOWER: [&str; 22] = [
    "...........#..........",
    "...........###........",
    "...........#####......",
    "...........###........",
    "...........#..........",
    "...........#..........",
    "......##..##..##......",
    "......##..##..##......",
    "......##..##..##......",
    "......##########......",
    "......##########......",
    "......###....###......",
    "......###....###......",
    "......##########......",
    "......####..####......",
    "......####..####......",
    "......####..####......",
    "......####..####......",
    "......####..####......",
    "....##############....",
    "....##############....",
    "......................",
];

/// Why the tray could not run.
#[derive(Debug, Error)]
pub enum TrayError {
    /// `AppKit` only works from the main thread.
    #[error("the tray must run on the main thread")]
    NotMainThread,
    /// The registry could not be read.
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// The icon or menu could not be created.
    #[error("menu bar: {0}")]
    Ui(String),
    /// The runtime used to poll daemons could not start.
    #[error("polling runtime: {0}")]
    Runtime(#[source] std::io::Error),
}

/// What a menu row does when clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    /// Open this dashboard.
    Open(String),
    /// SIGINT this daemon.
    Stop(u32),
    /// Exit the tray.
    Quit,
}

/// The menu as currently built, mapping row ids to actions.
#[derive(Default)]
struct Rows {
    items: Vec<Box<dyn IsMenuItem>>,
    actions: Vec<(MenuId, Action)>,
}

impl Rows {
    fn action(&self, id: &MenuId) -> Option<&Action> {
        self.actions
            .iter()
            .find(|(row, _)| row == id)
            .map(|(_, action)| action)
    }
}

/// Where the running tray records its pid, next to the daemon registry.
fn pid_file() -> Result<std::path::PathBuf, TrayError> {
    Ok(Registry::default_path()?.with_file_name("tray.pid"))
}

/// Whether the pid in the tray's pid file is a live process.
fn tray_running() -> bool {
    let Ok(path) = pid_file() else {
        return false;
    };
    let Some(pid) = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
    else {
        return false;
    };
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .is_ok_and(|s| s.success())
}

/// Starts `tirith tray` detached, unless one is already running (its pid
/// file names a live process). Called by `tirith serve` so the first
/// daemon on a machine brings the icon up.
pub fn launch_if_absent() -> Result<(), TrayError> {
    if tray_running() {
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(TrayError::Runtime)?;
    Command::new(exe)
        .arg("tray")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(TrayError::Runtime)?;
    Ok(())
}

/// Runs the tray until the user quits it. Must be called on the main
/// thread. Writes the pid file on start and removes it on quit; a second
/// tray started meanwhile exits at once.
pub fn run() -> Result<(), TrayError> {
    if tray_running() {
        return Ok(());
    }
    let pid_path = pid_file()?;
    if let Some(dir) = pid_path.parent() {
        std::fs::create_dir_all(dir).map_err(TrayError::Runtime)?;
    }
    std::fs::write(&pid_path, std::process::id().to_string()).map_err(TrayError::Runtime)?;
    let result = run_loop();
    let _ = std::fs::remove_file(&pid_path);
    result
}

fn run_loop() -> Result<(), TrayError> {
    let mtm = MainThreadMarker::new().ok_or(TrayError::NotMainThread)?;
    let app = NSApplication::sharedApplication(mtm);
    // No Dock icon, no menu bar of our own: a status item only.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let menu = Menu::new();
    // Kept for the loop's lifetime: dropping the icon removes it from the
    // menu bar.
    let _tray_icon = TrayIconBuilder::new()
        .with_icon(icon(2)?)
        .with_icon_as_template(true)
        .with_tooltip("Tirith daemons")
        .with_menu(Box::new(menu.clone()))
        .with_menu_on_left_click(true)
        .build()
        .map_err(|e| TrayError::Ui(e.to_string()))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(TrayError::Runtime)?;
    let mode = NSString::from_str("kCFRunLoopDefaultMode");
    let mut rows = Rows::default();
    let mut last_poll: Option<Instant> = None;
    // Human queue length per daemon pid at the last poll.
    let mut needs_you: HashMap<u32, usize> = HashMap::new();

    loop {
        if last_poll.is_none_or(|t| t.elapsed() >= POLL) {
            rows = refresh(&menu, &rows, &runtime, &mut needs_you)?;
            last_poll = Some(Instant::now());
        }
        let until = NSDate::dateWithTimeIntervalSinceNow(POLL.as_secs_f64());
        if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            NSEventMask::Any,
            Some(&until),
            &mode,
            true,
        ) {
            app.sendEvent(&event);
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            match rows.action(&event.id) {
                Some(Action::Open(url)) => open(url),
                Some(Action::Stop(pid)) => stop(*pid),
                Some(Action::Quit) => return Ok(()),
                None => {}
            }
        }
        // Clicks on the icon only open the menu; drain the channel so the
        // events do not pile up for as long as the tray runs.
        while TrayIconEvent::receiver().try_recv().is_ok() {}
    }
}

/// Re-reads the registry, drops dead daemons, rebuilds the menu, and
/// notifies when a daemon's human queue grew since the last poll.
fn refresh(
    menu: &Menu,
    old: &Rows,
    runtime: &tokio::runtime::Runtime,
    needs_you: &mut HashMap<u32, usize>,
) -> Result<Rows, TrayError> {
    let mut registry = Registry::load(Registry::default_path()?)?;
    let counts: Vec<(DaemonEntry, Option<Counts>)> = registry
        .entries()
        .iter()
        .map(|entry| (entry.clone(), runtime.block_on(counts(entry))))
        .collect();
    registry.prune(|entry| {
        counts
            .iter()
            .any(|(e, c)| e.pid == entry.pid && c.is_some())
    })?;
    for item in &old.items {
        let _ = menu.remove(item.as_ref());
    }
    let mut rows = Rows::default();
    let live: Vec<(&DaemonEntry, Counts)> = counts
        .iter()
        .filter_map(|(entry, counts)| counts.map(|c| (entry, c)))
        .collect();
    if live.is_empty() {
        let none = MenuItem::new("No Tirith daemons running", false, None);
        rows.actions.push((none.id().clone(), Action::Quit));
        append(menu, &mut rows, none)?;
    }
    let previous = std::mem::take(needs_you);
    for (entry, counts) in &live {
        if counts.needs_you > previous.get(&entry.pid).copied().unwrap_or(0) {
            notify(&entry.folder_name(), counts.needs_you);
        }
        needs_you.insert(entry.pid, counts.needs_you);
        let waiting = if counts.needs_you > 0 {
            format!(", {} need you", counts.needs_you)
        } else {
            String::new()
        };
        let label = format!(
            "{}  {} agents, {} claims{waiting}",
            entry.folder_name(),
            counts.agents,
            counts.claims
        );
        let open_row = MenuItem::new(label, true, None);
        rows.actions.push((
            open_row.id().clone(),
            Action::Open(entry.dashboard_url.clone()),
        ));
        append(menu, &mut rows, open_row)?;
        let stop_row = MenuItem::new(format!("    Stop {}", entry.folder_name()), true, None);
        rows.actions
            .push((stop_row.id().clone(), Action::Stop(entry.pid)));
        append(menu, &mut rows, stop_row)?;
    }
    let separator = PredefinedMenuItem::separator();
    menu.append(&separator)
        .map_err(|e| TrayError::Ui(e.to_string()))?;
    rows.items.push(Box::new(separator));
    let quit = MenuItem::new("Quit tray", true, None);
    rows.actions.push((quit.id().clone(), Action::Quit));
    append(menu, &mut rows, quit)?;
    Ok(rows)
}

fn append(menu: &Menu, rows: &mut Rows, item: MenuItem) -> Result<(), TrayError> {
    menu.append(&item)
        .map_err(|e| TrayError::Ui(e.to_string()))?;
    rows.items.push(Box::new(item));
    Ok(())
}

/// What a menu row shows for a daemon.
#[derive(Debug, Clone, Copy)]
struct Counts {
    agents: usize,
    claims: usize,
    /// Items in the daemon's human queue; 0 for daemons that predate it.
    needs_you: usize,
}

/// Asks a daemon for its state; `None` when it does not answer in time or
/// its repository is gone, which is what makes a crashed daemon vanish.
async fn counts(entry: &DaemonEntry) -> Option<Counts> {
    if !entry.root.is_dir() {
        return None;
    }
    let url = format!("{}api/state", entry.dashboard_url);
    let state: Value = reqwest::Client::new()
        .get(url)
        .timeout(DAEMON_TIMEOUT)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let claims = state["claims"].as_array()?;
    let agents: BTreeSet<&str> = claims.iter().filter_map(|c| c["owner"].as_str()).collect();
    Some(Counts {
        agents: agents.len(),
        claims: claims.len(),
        needs_you: state["needs_you"].as_array().map_or(0, Vec::len),
    })
}

/// Shows a macOS notification that `count` escalations in `folder` wait
/// for the human. Best effort: a failure is only logged.
fn notify(folder: &str, count: usize) {
    if let Err(error) = Command::new("osascript")
        .args(["-e", &notification_script(folder, count)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        tracing::warn!(%error, folder, "could not show a notification");
    }
}

/// The `AppleScript` for [`notify`], with both strings escaped.
fn notification_script(folder: &str, count: usize) -> String {
    let quote = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
    let body = if count == 1 {
        "1 escalation needs you".to_owned()
    } else {
        format!("{count} escalations need you")
    };
    format!(
        "display notification \"{}\" with title \"Tirith: {}\"",
        quote(&body),
        quote(folder)
    )
}

fn open(url: &str) {
    if let Err(error) = Command::new("open").arg(url).status() {
        tracing::warn!(%error, url, "could not open the dashboard");
    }
}

fn stop(pid: u32) {
    if let Err(error) = Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status()
    {
        tracing::warn!(%error, pid, "could not signal the daemon");
    }
}

/// The tower bitmap rasterized at `scale` as RGBA.
fn icon(scale: u32) -> Result<Icon, TrayError> {
    let side = 22 * scale;
    let mut rgba = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        let row = TOWER[(y / scale) as usize].as_bytes();
        for x in 0..side {
            let on = row.get((x / scale) as usize) == Some(&b'#');
            rgba.extend_from_slice(if on { &[0, 0, 0, 255] } else { &[0, 0, 0, 0] });
        }
    }
    Icon::from_rgba(rgba, side, side).map_err(|e| TrayError::Ui(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tower_bitmap_is_square_and_rasterizes_at_any_scale() {
        for row in TOWER {
            assert_eq!(row.len(), 22);
        }
        for scale in [1, 2] {
            let side = 22 * scale;
            let mut opaque = 0;
            for y in 0..side {
                let row = TOWER[(y / scale) as usize].as_bytes();
                for x in 0..side {
                    if row[(x / scale) as usize] == b'#' {
                        opaque += 1;
                    }
                }
            }
            assert!(
                opaque > 100 * (scale * scale) as usize,
                "the tower is drawn"
            );
            assert!(icon(scale).is_ok());
        }
    }

    #[test]
    fn the_notification_script_escapes_quotes_and_backslashes() {
        assert_eq!(
            notification_script("my \"repo\\x", 2),
            "display notification \"2 escalations need you\" with title \"Tirith: my \\\"repo\\\\x\""
        );
        assert!(notification_script("tirith", 1).contains("\"1 escalation needs you\""));
    }

    #[test]
    fn rows_map_ids_to_actions() {
        let mut rows = Rows::default();
        let item = MenuItem::new("x", true, None);
        rows.actions.push((item.id().clone(), Action::Stop(7)));
        assert_eq!(rows.action(item.id()), Some(&Action::Stop(7)));
        assert_eq!(rows.action(&MenuId::new("other")), None);
    }
}
