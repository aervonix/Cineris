//! The MCP client. Registers server tools with the dynamic-tool registry and reads other clients'
//! server definitions.
//!
//! SIZE PROBE ONLY (a scratch branch, never merged): a client on `rmcp`
//! that does what step 30's client does, reachable from the binary, so that
//! the shipped build's size with it can be measured.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rmcp::model::{CallToolRequestParams, ClientRequest, ProtocolVersion};
use rmcp::service::{ClientLifecycleMode, ClientServiceExt, NotificationContext, PeerRequestOptions};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ClientHandler, RoleClient};

#[derive(Clone, Default)]
struct Probe {
    changes: Arc<AtomicUsize>,
}

impl ClientHandler for Probe {
    fn on_tool_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.changes.fetch_add(1, Ordering::Relaxed);
        std::future::ready(())
    }
}

fn lifecycle() -> ClientLifecycleMode {
    ClientLifecycleMode::Auto {
        preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        legacy_version: Some(ProtocolVersion::V_2025_11_25),
    }
}

async fn exercise<S>(service: rmcp::service::RunningService<RoleClient, S>) -> Result<String, String>
where
    S: rmcp::service::Service<RoleClient>,
{
    let tools = service.list_all_tools().await.map_err(|e| e.to_string())?;
    let mut out = String::new();
    for tool in &tools {
        out.push_str(&tool.name);
        out.push_str(&serde_json::to_string(&tool.input_schema).unwrap_or_default());
    }
    if let Some(first) = tools.first() {
        let mut params = CallToolRequestParams::new(first.name.clone());
        params.arguments = Some(serde_json::Map::new());
        let result = service.call_tool(params.clone()).await.map_err(|e| e.to_string())?;
        out.push_str(&serde_json::to_string(&result).unwrap_or_default());
        let handle = service
            .send_cancellable_request(
                ClientRequest::CallToolRequest(rmcp::model::Request::new(params)),
                PeerRequestOptions::no_options(),
            )
            .await
            .map_err(|e| e.to_string())?;
        handle.cancel(Some("probe".to_owned())).await.map_err(|e| e.to_string())?;
    }
    service.cancel().await.map_err(|e| e.to_string())?;
    Ok(out)
}

/// Connects to a server over standard I/O (`program`) or streamable HTTP
/// (`url`), lists its tools, calls the first and cancels a second call.
pub async fn probe(
    program: Option<&str>,
    url: Option<&str>,
    token: Option<String>,
) -> Result<String, String> {
    let handler = Probe::default();
    if let Some(program) = program {
        // The harness's own launcher gives the pipes; tokio's stand in here.
        let mut child = tokio::process::Command::new(program)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let transport = (stdout, stdin);
        let service = handler
            .clone()
            .serve_with_lifecycle(transport, lifecycle())
            .await
            .map_err(|e| e.to_string())?;
        return exercise(service).await;
    }
    if let Some(url) = url {
        let mut headers = HashMap::new();
        headers.insert(
            reqwest::header::HeaderName::from_static("x-probe"),
            reqwest::header::HeaderValue::from_static("1"),
        );
        let mut config = StreamableHttpClientTransportConfig::with_uri(url).custom_headers(headers);
        if let Some(token) = token {
            config = config.auth_header(token);
        }
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| e.to_string())?;
        let transport = StreamableHttpClientTransport::with_client(client, config);
        let service = handler
            .serve_with_lifecycle(transport, lifecycle())
            .await
            .map_err(|e| e.to_string())?;
        return exercise(service).await;
    }
    Ok(String::new())
}
