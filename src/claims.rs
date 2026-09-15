//! Claims: leases on repository paths.
//!
//! An agent claims paths before editing them. A claim is a lease: it
//! expires after its TTL unless the agent renews it, and any call by the
//! owning agent renews all of its leases. Overlapping claims by different
//! agents are refused with enough information for the refused agent to
//! decide what to do. This module is pure; time comes in as an argument.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::{AgentId, ClaimId, RepoPath};

/// Lease length when the caller does not specify one.
pub const DEFAULT_TTL_SECS: u64 = 600;
/// Longest lease a caller may request.
pub const MAX_TTL_SECS: u64 = 3600;

/// One agent's lease on one or more paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Claim {
    /// Unique identifier of this claim.
    pub id: ClaimId,
    /// The agent holding the lease.
    pub owner: AgentId,
    /// Paths covered, each including everything beneath it.
    pub paths: Vec<RepoPath>,
    /// Why the agent wants these paths. Shown to whoever is refused.
    pub reason: String,
    /// When the claim was created.
    pub claimed_at: DateTime<Utc>,
    /// When the lease ends unless renewed.
    pub expires_at: DateTime<Utc>,
    /// Lease length used on every renewal.
    pub ttl_secs: u64,
}

impl Claim {
    /// Whether the lease has ended at `now`.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }

    /// Extends the lease to `now` plus the claim's TTL.
    pub fn renew(&mut self, now: DateTime<Utc>) {
        self.expires_at = now + ttl_duration(self.ttl_secs);
    }

    /// Whether any of this claim's paths overlaps `path`.
    pub fn covers(&self, path: &RepoPath) -> bool {
        self.paths.iter().any(|p| p.overlaps(path))
    }
}

/// One refused path and the claim it collides with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Conflict {
    /// The path that was requested.
    pub path: RepoPath,
    /// The existing claimed path it overlaps.
    pub overlaps: RepoPath,
    /// Who holds the existing claim.
    pub owner: AgentId,
    /// Their stated reason.
    pub reason: String,
    /// When their lease ends unless renewed.
    pub expires_at: DateTime<Utc>,
}

/// The result of a successful claim call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Granted {
    /// The new claim, if any path was not already held by the agent.
    pub claim_id: Option<ClaimId>,
    /// Paths newly claimed by this call.
    pub new_paths: Vec<RepoPath>,
    /// Paths the agent already held; their leases were renewed.
    pub renewed_paths: Vec<RepoPath>,
    /// When the newest lease ends.
    pub expires_at: DateTime<Utc>,
}

/// Why a claim operation was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClaimError {
    /// At least one requested path overlaps another agent's claim.
    #[error("{} path(s) overlap existing claims", .0.len())]
    Conflict(Vec<Conflict>),
    /// The agent tried to release paths it does not hold.
    #[error("{agent} does not hold {}", join(paths))]
    NotHeld {
        /// The releasing agent.
        agent: AgentId,
        /// The paths that were not held.
        paths: Vec<RepoPath>,
    },
    /// The agent holds nothing, so there is nothing to release or renew.
    #[error("{0} holds no claims")]
    NoClaims(AgentId),
    /// No paths were given.
    #[error("at least one path is required")]
    NoPaths,
    /// The requested TTL is outside the allowed range.
    #[error("ttl_secs must be between 1 and {MAX_TTL_SECS}, got {0}")]
    InvalidTtl(u64),
}

fn join(paths: &[RepoPath]) -> String {
    paths
        .iter()
        .map(RepoPath::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

fn ttl_duration(secs: u64) -> Duration {
    Duration::seconds(i64::try_from(secs).unwrap_or(i64::MAX))
}

/// All live claims. Expired claims are removed by [`ClaimBook::reap`], which
/// callers run before every read or write.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimBook {
    claims: Vec<Claim>,
}

impl ClaimBook {
    /// Rebuilds a book from persisted claims.
    pub fn from_claims(claims: Vec<Claim>) -> Self {
        Self { claims }
    }

    /// All claims, live or not; call [`reap`](Self::reap) first.
    pub fn claims(&self) -> &[Claim] {
        &self.claims
    }

    /// Removes expired claims and returns them.
    pub fn reap(&mut self, now: DateTime<Utc>) -> Vec<Claim> {
        let (expired, live): (Vec<_>, Vec<_>) =
            self.claims.drain(..).partition(|c| c.is_expired(now));
        self.claims = live;
        expired
    }

    /// Renews every lease held by `agent`. Returns how many were renewed.
    pub fn touch(&mut self, agent: &AgentId, now: DateTime<Utc>) -> usize {
        let mut count = 0;
        for claim in self.claims.iter_mut().filter(|c| &c.owner == agent) {
            claim.renew(now);
            count += 1;
        }
        count
    }

    /// Conflicts between `paths` and claims held by agents other than
    /// `agent`.
    pub fn conflicts_for(&self, agent: &AgentId, paths: &[RepoPath]) -> Vec<Conflict> {
        let mut conflicts = Vec::new();
        for path in paths {
            for claim in self.claims.iter().filter(|c| &c.owner != agent) {
                for held in claim.paths.iter().filter(|held| held.overlaps(path)) {
                    conflicts.push(Conflict {
                        path: path.clone(),
                        overlaps: held.clone(),
                        owner: claim.owner.clone(),
                        reason: claim.reason.clone(),
                        expires_at: claim.expires_at,
                    });
                }
            }
        }
        conflicts
    }

    /// Claims `paths` for `agent`, or refuses atomically if any path
    /// overlaps another agent's claim. Paths the agent already holds are
    /// renewed instead of duplicated.
    pub fn claim(
        &mut self,
        agent: AgentId,
        paths: Vec<RepoPath>,
        reason: String,
        ttl_secs: u64,
        now: DateTime<Utc>,
    ) -> Result<Granted, ClaimError> {
        if paths.is_empty() {
            return Err(ClaimError::NoPaths);
        }
        if ttl_secs == 0 || ttl_secs > MAX_TTL_SECS {
            return Err(ClaimError::InvalidTtl(ttl_secs));
        }
        let conflicts = self.conflicts_for(&agent, &paths);
        if !conflicts.is_empty() {
            return Err(ClaimError::Conflict(conflicts));
        }
        let (renewed_paths, new_paths): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| {
            self.claims.iter().any(|c| {
                c.owner == agent
                    && c.paths
                        .iter()
                        .any(|held| held.is_ancestor_of(p) || held == p)
            })
        });
        let mut expires_at = now;
        for claim in self.claims.iter_mut().filter(|c| c.owner == agent) {
            if renewed_paths.iter().any(|p| claim.covers(p)) {
                claim.renew(now);
                expires_at = expires_at.max(claim.expires_at);
            }
        }
        let claim_id = if new_paths.is_empty() {
            None
        } else {
            let claim = Claim {
                id: ClaimId::new(),
                owner: agent,
                paths: new_paths.clone(),
                reason,
                claimed_at: now,
                expires_at: now + ttl_duration(ttl_secs),
                ttl_secs,
            };
            expires_at = expires_at.max(claim.expires_at);
            let id = claim.id;
            self.claims.push(claim);
            Some(id)
        };
        Ok(Granted {
            claim_id,
            new_paths,
            renewed_paths,
            expires_at,
        })
    }

    /// Releases `paths` held by `agent`, or everything the agent holds when
    /// `paths` is `None`. Refuses atomically if any path is not held
    /// exactly as claimed.
    pub fn release(
        &mut self,
        agent: &AgentId,
        paths: Option<Vec<RepoPath>>,
    ) -> Result<Vec<RepoPath>, ClaimError> {
        let held: Vec<RepoPath> = self
            .claims
            .iter()
            .filter(|c| &c.owner == agent)
            .flat_map(|c| c.paths.iter().cloned())
            .collect();
        if held.is_empty() {
            return Err(ClaimError::NoClaims(agent.clone()));
        }
        let to_release = match paths {
            None => held,
            Some(requested) => {
                let missing: Vec<RepoPath> = requested
                    .iter()
                    .filter(|p| !held.contains(p))
                    .cloned()
                    .collect();
                if !missing.is_empty() {
                    return Err(ClaimError::NotHeld {
                        agent: agent.clone(),
                        paths: missing,
                    });
                }
                requested
            }
        };
        for claim in self.claims.iter_mut().filter(|c| &c.owner == agent) {
            claim.paths.retain(|p| !to_release.contains(p));
        }
        self.claims.retain(|c| !c.paths.is_empty());
        Ok(to_release)
    }

    /// Renews every lease held by `agent` and returns the renewed claims.
    pub fn renew(&mut self, agent: &AgentId, now: DateTime<Utc>) -> Result<Vec<Claim>, ClaimError> {
        if self.touch(agent, now) == 0 {
            return Err(ClaimError::NoClaims(agent.clone()));
        }
        Ok(self
            .claims
            .iter()
            .filter(|c| &c.owner == agent)
            .cloned()
            .collect())
    }

    /// Claims overlapping `path`, or all claims when `path` is `None`.
    pub fn list(&self, path: Option<&RepoPath>) -> Vec<&Claim> {
        self.claims
            .iter()
            .filter(|c| path.is_none_or(|p| c.covers(p)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn agent(name: &str) -> AgentId {
        AgentId::new(name).unwrap()
    }

    fn path(p: &str) -> RepoPath {
        RepoPath::new(p).unwrap()
    }

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 15, 18, 0, 0).unwrap()
    }

    #[test]
    fn directory_claim_blocks_file_inside_it() {
        let mut book = ClaimBook::default();
        book.claim(
            agent("alice"),
            vec![path("src/auth")],
            "refactor".into(),
            600,
            t0(),
        )
        .unwrap();
        let err = book
            .claim(
                agent("bob"),
                vec![path("src/auth/login.rs")],
                "fix".into(),
                600,
                t0(),
            )
            .unwrap_err();
        let ClaimError::Conflict(conflicts) = err else {
            panic!("expected conflict");
        };
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].owner, agent("alice"));
        assert_eq!(conflicts[0].overlaps, path("src/auth"));
        assert_eq!(conflicts[0].reason, "refactor");
    }

    #[test]
    fn conflict_is_atomic_across_paths() {
        let mut book = ClaimBook::default();
        book.claim(agent("alice"), vec![path("a")], "x".into(), 600, t0())
            .unwrap();
        let err = book.claim(
            agent("bob"),
            vec![path("b"), path("a/1")],
            "y".into(),
            600,
            t0(),
        );
        assert!(matches!(err, Err(ClaimError::Conflict(_))));
        assert!(book.list(Some(&path("b"))).is_empty());
    }

    #[test]
    fn sibling_directories_do_not_conflict() {
        let mut book = ClaimBook::default();
        book.claim(
            agent("alice"),
            vec![path("src/auth")],
            "x".into(),
            600,
            t0(),
        )
        .unwrap();
        assert!(
            book.claim(agent("bob"), vec![path("src/authz")], "y".into(), 600, t0())
                .is_ok()
        );
    }

    #[test]
    fn expired_claims_are_reaped_and_paths_become_free() {
        let mut book = ClaimBook::default();
        book.claim(agent("alice"), vec![path("a")], "x".into(), 60, t0())
            .unwrap();
        let later = t0() + Duration::seconds(61);
        let expired = book.reap(later);
        assert_eq!(expired.len(), 1);
        assert!(
            book.claim(agent("bob"), vec![path("a")], "y".into(), 60, later)
                .is_ok()
        );
    }

    #[test]
    fn touch_renews_only_the_agents_claims() {
        let mut book = ClaimBook::default();
        book.claim(agent("alice"), vec![path("a")], "x".into(), 60, t0())
            .unwrap();
        book.claim(agent("bob"), vec![path("b")], "y".into(), 60, t0())
            .unwrap();
        let later = t0() + Duration::seconds(30);
        assert_eq!(book.touch(&agent("alice"), later), 1);
        let alice = &book.list(Some(&path("a")))[0];
        let bob = &book.list(Some(&path("b")))[0];
        assert_eq!(alice.expires_at, later + Duration::seconds(60));
        assert_eq!(bob.expires_at, t0() + Duration::seconds(60));
    }

    #[test]
    fn reclaiming_held_path_renews_instead_of_duplicating() {
        let mut book = ClaimBook::default();
        book.claim(agent("alice"), vec![path("a")], "x".into(), 60, t0())
            .unwrap();
        let later = t0() + Duration::seconds(10);
        let granted = book
            .claim(
                agent("alice"),
                vec![path("a/b"), path("c")],
                "x".into(),
                60,
                later,
            )
            .unwrap();
        assert_eq!(granted.renewed_paths, vec![path("a/b")]);
        assert_eq!(granted.new_paths, vec![path("c")]);
        assert_eq!(book.claims().len(), 2);
    }

    #[test]
    fn release_is_exact_and_atomic() {
        let mut book = ClaimBook::default();
        book.claim(
            agent("alice"),
            vec![path("a"), path("b")],
            "x".into(),
            60,
            t0(),
        )
        .unwrap();
        let err = book
            .release(&agent("alice"), Some(vec![path("a"), path("zzz")]))
            .unwrap_err();
        assert!(matches!(err, ClaimError::NotHeld { .. }));
        assert_eq!(book.claims()[0].paths.len(), 2);
        let released = book
            .release(&agent("alice"), Some(vec![path("a")]))
            .unwrap();
        assert_eq!(released, vec![path("a")]);
        assert_eq!(book.claims()[0].paths, vec![path("b")]);
        book.release(&agent("alice"), None).unwrap();
        assert!(book.claims().is_empty());
        assert!(matches!(
            book.release(&agent("alice"), None),
            Err(ClaimError::NoClaims(_))
        ));
    }

    #[test]
    fn ttl_is_bounded() {
        let mut book = ClaimBook::default();
        assert_eq!(
            book.claim(agent("a"), vec![path("x")], String::new(), 0, t0()),
            Err(ClaimError::InvalidTtl(0))
        );
        assert_eq!(
            book.claim(
                agent("a"),
                vec![path("x")],
                String::new(),
                MAX_TTL_SECS + 1,
                t0()
            ),
            Err(ClaimError::InvalidTtl(MAX_TTL_SECS + 1))
        );
        assert_eq!(
            book.claim(agent("a"), vec![], String::new(), 60, t0()),
            Err(ClaimError::NoPaths)
        );
    }
}
