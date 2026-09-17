//! `tirith tray`: a macOS menu bar icon listing every Tirith daemon on
//! this machine, read from the per-user registry (`registry.rs`). Clicking
//! the icon always shows the menu: one row per daemon with its agent and
//! claim counts, and how many items wait for the human (the daemon's human
//! queue, ADR-0027), then the first line of each item and who sent it.
//! Clicking a daemon row opens its dashboard, an item row opens the
//! dashboard's Needs you section, `Stop` sends the daemon SIGINT, `Quit
//! tray` exits. See ADR-0019. A daemon that does not answer in time stays
//! listed as not responding; the tray drops it from the registry only once
//! its process or its repository is gone. When a new item reaches a daemon's human
//! queue, a macOS notification names its sender and first line (through
//! `osascript`, so no notification entitlement or app bundle is needed).
//!
//! One tray runs per state directory: it holds an OS lock on `tray.lock`
//! beside the registry for its whole life, and a second tray that cannot
//! take the lock exits at once (ADR-0032).
//!
//! `AppKit` needs its event loop on the main thread. Rather than pull in
//! `winit`, this module pumps the loop by hand: wait for an event or a
//! five-second timeout, dispatch it, drain the menu and tray channels,
//! refresh the registry. Everything is the safe surface of `objc2`.

use std::collections::{BTreeSet, HashMap};
use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
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
/// not responding for this poll.
const DAEMON_TIMEOUT: Duration = Duration::from_millis(800);

/// Consecutive unanswered polls (five minutes) after which a daemon whose
/// pid still exists is dropped anyway: the pid then most likely belongs to
/// another process now. A daemon that was only busy puts its entry back
/// within a minute (`registry::Registration`).
const GIVE_UP_POLLS: u32 = 60;

/// Characters of a human queue item's first line shown in a menu row or a
/// notification.
const LINE_CHARS: usize = 60;

/// Human queue items listed under one daemon; the rest are counted.
const NEEDS_SHOWN: usize = 5;

/// The dashboard section a human queue item row opens.
const NEEDS_ANCHOR: &str = "#s-needs";

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
    /// The single-instance lock file could not be opened or locked.
    #[error("tray lock {path}: {source}")]
    Lock {
        /// The lock file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
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

/// The lock a running tray holds, next to the daemon registry.
fn lock_path() -> Result<PathBuf, TrayError> {
    Ok(Registry::default_path()?.with_file_name("tray.lock"))
}

/// Takes the single-instance lock at `path` without waiting. It is an OS
/// advisory lock (`flock`) on the file, held for as long as the returned
/// handle lives; the kernel drops it when the process exits, however it
/// exits, so a crashed tray never leaves a stale lock behind and a reused
/// pid means nothing. `None`: another open handle holds it.
fn try_lock(path: &Path) -> Result<Option<File>, TrayError> {
    let lock_err = |source| TrayError::Lock {
        path: path.to_path_buf(),
        source,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(lock_err)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(lock_err)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(source)) => Err(lock_err(source)),
    }
}

/// Starts `tirith tray` detached, unless a tray already holds the lock.
/// Called by `tirith serve` so the first daemon on a machine brings the
/// icon up. Two daemons starting at once may both spawn one; the second
/// tray finds the lock taken and exits at once, so one icon remains.
pub fn launch_if_absent() -> Result<(), TrayError> {
    let Some(probe) = try_lock(&lock_path()?)? else {
        return Ok(());
    };
    // Let go before spawning, or the new tray would find the lock taken.
    drop(probe);
    let exe = std::env::current_exe().map_err(TrayError::Runtime)?;
    let mut command = Command::new(exe);
    command
        .arg("tray")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // Its own process group: a ctrl-c in the terminal of the daemon
        // that launched it must not take the icon down with the daemon.
        .process_group(0);
    command.spawn().map_err(TrayError::Runtime)?;
    Ok(())
}

/// Runs the tray until the user quits it. Must be called on the main
/// thread. Holds the single-instance lock for its whole life; a second
/// tray finds it taken and returns at once.
pub fn run() -> Result<(), TrayError> {
    let Some(_lock) = try_lock(&lock_path()?)? else {
        return Ok(());
    };
    run_loop()
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
    // Human queue item ids per daemon pid at the last poll.
    let mut needs_you: HashMap<u32, BTreeSet<u64>> = HashMap::new();
    // Consecutive unanswered polls per daemon pid.
    let mut misses: HashMap<u32, u32> = HashMap::new();

    loop {
        if last_poll.is_none_or(|t| t.elapsed() >= POLL) {
            rows = refresh(&menu, &rows, &runtime, &mut needs_you, &mut misses)?;
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

/// Re-reads the registry, drops daemons that are gone, rebuilds the menu,
/// and notifies about human queue items that were not there at the last
/// poll.
fn refresh(
    menu: &Menu,
    old: &Rows,
    runtime: &tokio::runtime::Runtime,
    needs_you: &mut HashMap<u32, BTreeSet<u64>>,
    misses: &mut HashMap<u32, u32>,
) -> Result<Rows, TrayError> {
    let mut registry = Registry::load(Registry::default_path()?)?;
    let entries = registry.entries().to_vec();
    let answers = runtime.block_on(ask_all(&entries));
    misses.retain(|pid, _| entries.iter().any(|e| e.pid == *pid));
    let health: Vec<(DaemonEntry, Health)> = entries
        .into_iter()
        .zip(answers)
        .map(|(entry, answer)| {
            let root_exists = entry.root.is_dir();
            let health = judge(&entry, answer, root_exists, misses, pid_alive);
            (entry, health)
        })
        .collect();
    // Drop only the daemons this poll found gone: one that registered while
    // the others were being asked is kept for the next poll.
    registry.prune(|entry| {
        !health
            .iter()
            .any(|(e, h)| e.pid == entry.pid && *h == Health::Gone)
    })?;
    for item in &old.items {
        let _ = menu.remove(item.as_ref());
    }
    let mut rows = Rows::default();
    let listed: Vec<(&DaemonEntry, Option<&Counts>)> = health
        .iter()
        .filter_map(|(entry, health)| match health {
            Health::Live(counts) => Some((entry, Some(counts))),
            Health::NotResponding => Some((entry, None)),
            Health::Gone => None,
        })
        .collect();
    if listed.is_empty() {
        let none = MenuItem::new("No Tirith daemons running", false, None);
        rows.actions.push((none.id().clone(), Action::Quit));
        append(menu, &mut rows, none)?;
    }
    let mut previous = std::mem::take(needs_you);
    for (entry, counts) in listed {
        let open_row = MenuItem::new(daemon_label(&entry.folder_name(), counts), true, None);
        rows.actions.push((
            open_row.id().clone(),
            Action::Open(entry.dashboard_url.clone()),
        ));
        append(menu, &mut rows, open_row)?;
        match counts {
            Some(counts) => {
                let seen = previous.get(&entry.pid);
                let new: Vec<&Need> = counts
                    .needs
                    .iter()
                    .filter(|n| seen.is_none_or(|s| !s.contains(&n.id)))
                    .collect();
                if let Some(body) = notification_body(&new) {
                    notify(&entry.folder_name(), &body);
                }
                needs_you.insert(entry.pid, counts.needs.iter().map(|n| n.id).collect());
                let needs_url = format!("{}{NEEDS_ANCHOR}", entry.dashboard_url);
                for label in need_labels(&counts.needs) {
                    let need_row = MenuItem::new(label, true, None);
                    rows.actions
                        .push((need_row.id().clone(), Action::Open(needs_url.clone())));
                    append(menu, &mut rows, need_row)?;
                }
            }
            // Remember what it had, so its items do not notify again when
            // it answers.
            None => {
                if let Some(seen) = previous.remove(&entry.pid) {
                    needs_you.insert(entry.pid, seen);
                }
            }
        }
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

/// The row for a daemon: its folder and counts, or that it is not
/// responding when it did not answer this poll.
fn daemon_label(folder: &str, counts: Option<&Counts>) -> String {
    let Some(counts) = counts else {
        return format!("{folder}  not responding");
    };
    let waiting = if counts.needs.is_empty() {
        String::new()
    } else {
        format!(", {} need you", counts.needs.len())
    };
    format!(
        "{folder}  {} agents, {} claims{waiting}",
        counts.agents, counts.claims
    )
}

/// What one poll found out about a daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Health {
    /// It answered.
    Live(Counts),
    /// It did not answer in time, but its process still exists: listed,
    /// and kept in the registry.
    NotResponding,
    /// Its process or its repository is gone: dropped from the registry.
    Gone,
}

/// Judges a daemon from this poll's `answer` (`None`: none within
/// [`DAEMON_TIMEOUT`]). `misses` counts consecutive unanswered polls per
/// pid. `pid_alive` is asked only when the daemon did not answer: a slow
/// daemon is not a dead one, so the registry keeps it until its pid is gone
/// or it stayed silent for [`GIVE_UP_POLLS`].
fn judge(
    entry: &DaemonEntry,
    answer: Option<Counts>,
    root_exists: bool,
    misses: &mut HashMap<u32, u32>,
    pid_alive: impl Fn(u32) -> bool,
) -> Health {
    if !root_exists {
        misses.remove(&entry.pid);
        return Health::Gone;
    }
    if let Some(counts) = answer {
        misses.remove(&entry.pid);
        return Health::Live(counts);
    }
    let silent_polls = misses.entry(entry.pid).or_insert(0);
    *silent_polls += 1;
    if *silent_polls >= GIVE_UP_POLLS || !pid_alive(entry.pid) {
        misses.remove(&entry.pid);
        return Health::Gone;
    }
    Health::NotResponding
}

/// Whether process `pid` exists and is not a zombie, read from `ps`, which
/// only reads the process table and signals nothing. When `ps` cannot run
/// the answer is yes, so an entry is never dropped on a guess.
fn pid_alive(pid: u32) -> bool {
    match Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(out) => alive_from_ps(out.status.success(), &out.stdout),
        Err(error) => {
            tracing::warn!(%error, pid, "could not run ps");
            true
        }
    }
}

/// Reads `ps -o stat= -p <pid>`: it exits non-zero with no output when no
/// such process exists, and a zombie's state starts with `Z`.
fn alive_from_ps(success: bool, stdout: &[u8]) -> bool {
    let stat = String::from_utf8_lossy(stdout);
    let stat = stat.trim();
    success && !stat.is_empty() && !stat.starts_with('Z')
}

/// Asks every daemon for its state at once, so daemons that do not answer
/// cost one [`DAEMON_TIMEOUT`] per poll between them, not one each; the
/// answers are in `entries` order.
async fn ask_all(entries: &[DaemonEntry]) -> Vec<Option<Counts>> {
    let client = reqwest::Client::new();
    let asks: Vec<_> = entries
        .iter()
        .map(|entry| {
            let (client, url) = (client.clone(), format!("{}api/state", entry.dashboard_url));
            tokio::spawn(async move { counts(&client, url).await })
        })
        .collect();
    let mut answers = Vec::with_capacity(asks.len());
    for ask in asks {
        answers.push(ask.await.ok().flatten());
    }
    answers
}

fn append(menu: &Menu, rows: &mut Rows, item: MenuItem) -> Result<(), TrayError> {
    menu.append(&item)
        .map_err(|e| TrayError::Ui(e.to_string()))?;
    rows.items.push(Box::new(item));
    Ok(())
}

/// What a menu row shows for a daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Counts {
    agents: usize,
    claims: usize,
    /// The daemon's human queue, most urgent first; empty for daemons
    /// that predate it.
    needs: Vec<Need>,
}

/// One open human queue item as the tray shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Need {
    /// The item's id, stable while it stays open.
    id: u64,
    /// Who sent it or escalated.
    from: String,
    /// Its text's first non-blank line, cut to [`LINE_CHARS`].
    line: String,
}

/// The human queue items in a daemon's `/api/state` body (`needs_you`).
fn needs_of(state: &Value) -> Vec<Need> {
    state["needs_you"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(Need {
                        id: item["id"].as_u64()?,
                        from: item["agent"].as_str().unwrap_or("?").to_owned(),
                        line: first_line(item["text"].as_str().unwrap_or("")),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The first non-blank line of `text`, cut to [`LINE_CHARS`] characters
/// with an ellipsis when cut.
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() <= LINE_CHARS {
        return line.to_owned();
    }
    let mut cut: String = line.chars().take(LINE_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// The menu rows under a daemon: one per item, up to [`NEEDS_SHOWN`], then
/// how many more.
fn need_labels(needs: &[Need]) -> Vec<String> {
    let mut labels: Vec<String> = needs
        .iter()
        .take(NEEDS_SHOWN)
        .map(|n| format!("    {}: {}", n.from, n.line))
        .collect();
    if needs.len() > NEEDS_SHOWN {
        labels.push(format!("    +{} more", needs.len() - NEEDS_SHOWN));
    }
    labels
}

/// A notification body for items new since the last poll: the newest
/// one's sender and first line, and how many others arrived with it.
/// `None` when nothing is new.
fn notification_body(new: &[&Need]) -> Option<String> {
    let newest = new.iter().max_by_key(|n| n.id)?;
    let others = new.len() - 1;
    let more = if others > 0 {
        format!(" (+{others} more)")
    } else {
        String::new()
    };
    Some(format!("{}: {}{more}", newest.from, newest.line))
}

/// Asks a daemon for its state at `url`; `None` when it does not answer
/// within [`DAEMON_TIMEOUT`] or answers something else.
async fn counts(client: &reqwest::Client, url: String) -> Option<Counts> {
    let state: Value = client
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
        needs: needs_of(&state),
    })
}

/// Shows a macOS notification titled with `folder` that says `body`.
/// Best effort: a failure is only logged.
fn notify(folder: &str, body: &str) {
    if let Err(error) = Command::new("osascript")
        .args(["-e", &notification_script(folder, body)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        tracing::warn!(%error, folder, "could not show a notification");
    }
}

/// The `AppleScript` for [`notify`], with both strings escaped.
fn notification_script(folder: &str, body: &str) -> String {
    let quote = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "display notification \"{}\" with title \"Tirith: {}\"",
        quote(body),
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
            notification_script("my \"repo\\x", "lead: say \"yes\""),
            "display notification \"lead: say \\\"yes\\\"\" with title \"Tirith: my \\\"repo\\\\x\""
        );
    }

    #[test]
    fn items_show_their_sender_and_first_line_in_rows_and_notifications() {
        let long = "x".repeat(LINE_CHARS + 10);
        let state = serde_json::json!({ "needs_you": [
            { "id": 4, "agent": "swarm-lead", "text": "\n  The headless CLI is not logged in.\nRun claude login." },
            { "id": 9, "agent": "worker", "text": long },
            { "agent": "no id is skipped", "text": "x" },
        ]});
        let needs = needs_of(&state);
        assert_eq!(needs.len(), 2);
        assert_eq!(needs[0].line, "The headless CLI is not logged in.");
        assert_eq!(needs[1].line.chars().count(), LINE_CHARS);
        assert!(needs[1].line.ends_with('…'));
        assert_eq!(
            need_labels(&needs[..1]),
            ["    swarm-lead: The headless CLI is not logged in."]
        );
        assert!(needs_of(&serde_json::json!({})).is_empty(), "older daemons");

        let many: Vec<Need> = (0..7)
            .map(|id| Need {
                id,
                from: "w".into(),
                line: "l".into(),
            })
            .collect();
        let labels = need_labels(&many);
        assert_eq!(labels.len(), NEEDS_SHOWN + 1);
        assert_eq!(labels[NEEDS_SHOWN], "    +2 more");

        assert_eq!(notification_body(&[]), None);
        assert_eq!(
            notification_body(&[&needs[0]]).as_deref(),
            Some("swarm-lead: The headless CLI is not logged in.")
        );
        let body = notification_body(&[&needs[0], &needs[1]]).unwrap();
        assert!(body.starts_with("worker: xxx"), "the newest item: {body}");
        assert!(body.ends_with(" (+1 more)"), "{body}");
    }

    #[test]
    fn a_second_tray_lock_fails_while_the_first_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/tray.lock");
        let first = try_lock(&path).unwrap().expect("the first tray locks");
        assert!(
            try_lock(&path).unwrap().is_none(),
            "a second handle is refused while the first is open"
        );
        drop(first);
        let again = try_lock(&path).unwrap();
        assert!(again.is_some(), "free once the holder is gone");
    }

    fn daemon(pid: u32) -> DaemonEntry {
        DaemonEntry {
            root: PathBuf::from("/repos/busy"),
            url: "http://127.0.0.1:7477/mcp".into(),
            dashboard_url: "http://127.0.0.1:7477/".into(),
            pid,
            version: "1.0.0".into(),
            started_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn a_silent_daemon_is_dropped_only_once_its_process_or_repository_is_gone() {
        let busy = daemon(41);
        let counts = Counts {
            agents: 2,
            claims: 3,
            needs: Vec::new(),
        };
        let alive = |_| true;
        let dead = |_| false;
        let not_asked = |_| -> bool { unreachable!("an answering daemon's pid is not checked") };
        let mut misses = HashMap::new();

        let answered = judge(&busy, Some(counts.clone()), true, &mut misses, not_asked);
        assert_eq!(answered, Health::Live(counts.clone()));

        // Silent with its process alive: listed as not responding, poll after
        // poll, and an answer starts the count over.
        for _ in 0..GIVE_UP_POLLS - 1 {
            assert_eq!(
                judge(&busy, None, true, &mut misses, alive),
                Health::NotResponding
            );
        }
        let answered = judge(&busy, Some(counts.clone()), true, &mut misses, not_asked);
        assert_eq!(answered, Health::Live(counts.clone()));
        assert!(misses.is_empty());
        for _ in 0..GIVE_UP_POLLS - 1 {
            judge(&busy, None, true, &mut misses, alive);
        }
        // Silent for the whole grace period: the pid is someone else's now.
        assert_eq!(judge(&busy, None, true, &mut misses, alive), Health::Gone);
        assert!(misses.is_empty());

        // Silent with its process gone: dropped at once.
        assert_eq!(judge(&busy, None, true, &mut misses, dead), Health::Gone);
        // Its repository gone: dropped even if it still answers.
        assert_eq!(
            judge(&busy, Some(counts), false, &mut misses, not_asked),
            Health::Gone
        );
        assert!(misses.is_empty());
    }

    #[test]
    fn ps_output_tells_a_live_process_from_a_missing_or_zombie_one() {
        assert!(alive_from_ps(true, b"Ss\n"));
        assert!(alive_from_ps(true, b"  R+ "));
        assert!(!alive_from_ps(false, b""), "no such process");
        assert!(!alive_from_ps(true, b""));
        assert!(!alive_from_ps(true, b"Z+\n"), "a zombie is dead");
    }

    #[test]
    fn a_daemon_row_shows_counts_or_that_it_is_not_responding() {
        let mut counts = Counts {
            agents: 2,
            claims: 3,
            needs: Vec::new(),
        };
        assert_eq!(
            daemon_label("tirith", Some(&counts)),
            "tirith  2 agents, 3 claims"
        );
        counts.needs.push(Need {
            id: 1,
            from: "lead".into(),
            line: "x".into(),
        });
        assert_eq!(
            daemon_label("tirith", Some(&counts)),
            "tirith  2 agents, 3 claims, 1 need you"
        );
        assert_eq!(daemon_label("tirith", None), "tirith  not responding");
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
