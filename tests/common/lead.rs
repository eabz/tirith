//! The lead lease, for the test files whose daemon has a swarm lead.
//! Pulled in with `#[path]` by those files only (see `common/mod.rs`).

use serde_json::json;
use tirith::server::ServerHandle;

use crate::common::call;

/// `agent` takes the lead lease on `.tirith/lead` (ADR-0027).
pub(crate) async fn claim_lead(handle: &ServerHandle, agent: &str) {
    let out = call(
        handle,
        "claim",
        json!({ "agent": agent, "paths": [".tirith/lead"], "reason": "swarm", "brief": false }),
    )
    .await;
    assert_eq!(out["status"], "ok", "{out}");
}
