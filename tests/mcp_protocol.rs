//! Bounded, hermetic MCP protocol-contract test.
//!
//! Starts the real [`BrainServer`] over rmcp's in-process [`tokio::io::duplex`]
//! transport: no child process, no network, no external source, and no sync.
//! The duplex buffer is bounded and the whole conversation is wrapped in a
//! timeout, so a protocol regression fails the test instead of hanging.

use std::sync::Arc;
use std::time::Duration;

use cortana::embed::{DeterministicEmbedder, Embedder};
use cortana::mcp::BrainServer;
use cortana::store::Store;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
};
use serde_json::Value;
use tempfile::tempdir;

const DUPLEX_BUFFER_BYTES: usize = 64 * 1024;
const CONVERSATION_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test]
async fn protocol_contract_exposes_native_memory_tools_and_serves_brain_status()
-> anyhow::Result<()> {
    let directory = tempdir().expect("temporary directory");
    let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
    let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(16));

    // The two ends of one bounded in-process duplex stream; nothing leaves this
    // process. Each end already implements AsyncRead + AsyncWrite, which rmcp
    // accepts as a transport for either role.
    let (server_io, client_io) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);

    let server_handle = tokio::spawn(async move {
        BrainServer::new(store, embedder)
            .serve(server_io)
            .await?
            .waiting()
            .await?;
        anyhow::Ok(())
    });

    let outcome = tokio::time::timeout(CONVERSATION_TIMEOUT, async {
        // serve_client performs the MCP initialize handshake: it sends
        // initialize, validates the initialize result, then notifies the server
        // with notifications/initialized before returning.
        let client = ().serve(client_io).await?;

        // tools/list with cursor pagination; list_all_tools follows any pages.
        let tools = client.list_all_tools().await?;
        let mut names: Vec<String> = tools.iter().map(|tool| tool.name.to_string()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "brain_status",
                "context",
                "export_memory",
                "forget",
                "recall",
                "remember",
                "search",
                "search_code",
                "search_messages",
                "who_knows",
            ],
            "the public MCP tool surface changed; update this contract test deliberately"
        );

        // tools/call for brain_status must succeed with a structured JSON text result.
        let result: CallToolResult = client
            .call_tool(CallToolRequestParams::new("brain_status"))
            .await?;
        assert_ne!(result.is_error, Some(true), "brain_status tool call failed");
        let text = result
            .content
            .iter()
            .filter_map(|content| content.as_text())
            .map(|text| text.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        assert!(!text.is_empty(), "brain_status returned no text content");
        let stats: Value =
            serde_json::from_str(&text).expect("brain_status must return JSON store stats");
        assert_eq!(
            stats["documents"], 0,
            "fresh store must report zero documents"
        );
        assert!(
            stats.get("embedding_fingerprint").is_some(),
            "brain_status must report the embedding fingerprint"
        );

        client.cancel().await?;
        anyhow::Ok(())
    })
    .await;

    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(error),
        Err(_) => anyhow::bail!("MCP conversation exceeded {CONVERSATION_TIMEOUT:?}"),
    }

    // The server task finishes once the client cancels and the duplex closes.
    server_handle.await??;
    Ok(())
}
