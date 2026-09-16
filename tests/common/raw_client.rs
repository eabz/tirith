//! A bare rmcp client, for tests that need the raw `CallToolResult` or
//! the raw `tools/list` rather than the structured value `tirith::client`
//! picks. Pulled in with `#[path = "common/raw_client.rs"]` only by the
//! files that use it (see `common/mod.rs` for why).

use std::sync::Arc;

use rmcp::ServiceExt;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::common::client_side_sse::NeverRetry;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};

/// Connects one MCP session to `url` and completes the handshake.
pub(crate) async fn raw_client(url: &str) -> RunningService<RoleClient, ()> {
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_owned());
    config.retry_config = Arc::new(NeverRetry::default());
    let transport = StreamableHttpClientTransport::with_client(reqwest::Client::default(), config);
    ().serve(transport).await.unwrap()
}
