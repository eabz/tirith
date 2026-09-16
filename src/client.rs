//! A thin MCP client used by the CLI and the integration tests.
//!
//! It speaks streamable HTTP to a running daemon and returns each tool's
//! structured content as JSON, so the CLI exercises exactly the path an
//! agent uses.

use std::error::Error;
use std::sync::Arc;

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::common::client_side_sse::NeverRetry;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde_json::{Map, Value};
use thiserror::Error;

/// Why a client call failed.
#[derive(Debug, Error)]
pub enum ClientError {
    /// The daemon could not be reached or refused to initialize.
    #[error("cannot connect to {url} (is `tirith serve` running there?): {}", summarize(cause.as_ref()))]
    Connect {
        /// The MCP endpoint.
        url: String,
        /// The underlying error, kept for debugging but not chained in
        /// `Display` because rmcp's message repeats itself.
        cause: Box<dyn Error + Send + Sync>,
    },
    /// The tool call itself failed at the protocol level.
    #[error("calling {tool}: {source}")]
    Call {
        /// The tool name.
        tool: String,
        /// The underlying error.
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    /// Tool arguments must be a JSON object.
    #[error("tool arguments must be a JSON object")]
    ArgumentsNotObject,
    /// The server reported a tool execution error without structured
    /// content.
    #[error("{tool} failed: {message}")]
    ToolError {
        /// The tool name.
        tool: String,
        /// The server's text.
        message: String,
    },
}

/// The first clause of a transport error, without rmcp's repeated context.
fn summarize(error: &(dyn Error + Send + Sync)) -> String {
    let text = error.to_string();
    let clause = text.split(", when ").next().unwrap_or(&text);
    clause
        .rsplit("error: ")
        .next()
        .unwrap_or(clause)
        .trim()
        .to_owned()
}

/// A one-shot transport: no reconnect retries, so an unreachable daemon
/// fails immediately instead of backing off.
fn transport(url: &str) -> StreamableHttpClientTransport<reqwest::Client> {
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_owned());
    config.retry_config = Arc::new(NeverRetry::default());
    StreamableHttpClientTransport::with_client(reqwest::Client::default(), config)
}

/// An initialized MCP session with the daemon at `url`.
async fn connect(url: &str) -> Result<RunningService<RoleClient, ()>, ClientError> {
    ().serve(transport(url))
        .await
        .map_err(|e| ClientError::Connect {
            url: url.to_owned(),
            cause: Box::new(e),
        })
}

/// Calls `tool` on the daemon at `url` and returns its structured result.
///
/// `arguments` must be a JSON object or `null`. When the server returns no
/// structured content, the text content is returned as a JSON string.
pub async fn call_tool(url: &str, tool: &str, arguments: Value) -> Result<Value, ClientError> {
    let arguments: Map<String, Value> = match arguments {
        Value::Object(map) => map,
        Value::Null => Map::new(),
        _ => return Err(ClientError::ArgumentsNotObject),
    };
    let client = connect(url).await?;
    let result = client
        .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments))
        .await;
    let _ = client.cancel().await;
    let result = result.map_err(|e| ClientError::Call {
        tool: tool.to_owned(),
        source: Box::new(e),
    })?;
    if let Some(structured) = result.structured_content {
        return Ok(structured);
    }
    let text: Vec<String> = result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|t| t.text.clone()))
        .collect();
    let message = text.join("\n");
    if result.is_error.unwrap_or(false) {
        return Err(ClientError::ToolError {
            tool: tool.to_owned(),
            message,
        });
    }
    Ok(Value::String(message))
}

/// Lists the tools the daemon at `url` exposes, as `(name, description)`.
pub async fn list_tools(url: &str) -> Result<Vec<(String, String)>, ClientError> {
    let client = connect(url).await?;
    let result = client.list_all_tools().await;
    let _ = client.cancel().await;
    let tools = result.map_err(|e| ClientError::Call {
        tool: "tools/list".to_owned(),
        source: Box::new(e),
    })?;
    Ok(tools
        .into_iter()
        .map(|t| {
            (
                t.name.to_string(),
                t.description.map(|d| d.to_string()).unwrap_or_default(),
            )
        })
        .collect())
}
