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
                "cancel_memory_candidate",
                "classify_memory_candidate",
                "code_relations",
                "consolidate_memory_candidate",
                "context",
                "export_memory",
                "export_memory_candidates",
                "forget",
                "inspect_memory_representations",
                "list_memory_candidates",
                "lookup_symbol",
                "propose_memory_candidate",
                "provider_capabilities",
                "recall",
                "redact_memory_candidate",
                "reflect_memory",
                "remember",
                "search",
                "search_code",
                "search_messages",
                "who_knows",
            ],
            "the public MCP tool surface changed; update this contract test deliberately"
        );

        let remember = tools
            .iter()
            .find(|tool| tool.name == "remember")
            .expect("remember tool must be present");
        let remember_schema = serde_json::to_value(&*remember.input_schema)?;
        assert!(
            remember_schema["properties"]["provenance"].is_object(),
            "remember.provenance must be an object schema so strict MCP clients accept tools/list"
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

        let result: CallToolResult = client
            .call_tool(CallToolRequestParams::new("provider_capabilities"))
            .await?;
        assert_ne!(result.is_error, Some(true));
        let text = result
            .content
            .iter()
            .filter_map(|content| content.as_text())
            .map(|text| text.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        let capabilities: Value = serde_json::from_str(&text)?;
        assert_eq!(capabilities["contract_version"], "cortana.provider.v1");

        let expires_at = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let candidate_arguments = serde_json::json!({
            "observation_kind": "evidence-backed",
            "content_type": "semantic",
            "retention_tier": "working",
            "scope": "workspace",
            "project": "work",
            "title": "MCP candidate",
            "content": "Review this bounded candidate",
            "source": "mcp-test",
            "source_id": "candidate-1",
            "dedupe_key": "mcp-candidate-1",
            "confidence": 0.8,
            "importance": 0.5,
            "sensitivity": "normal",
            "acl": ["work"],
            "provenance": {"test": true},
            "expires_at": expires_at
        })
        .as_object()
        .expect("candidate arguments")
        .clone();
        let proposed = client
            .call_tool(
                CallToolRequestParams::new("propose_memory_candidate")
                    .with_arguments(candidate_arguments),
            )
            .await?;
        assert_ne!(proposed.is_error, Some(true), "candidate proposal failed");
        let proposed_text = proposed
            .content
            .iter()
            .filter_map(|content| content.as_text())
            .map(|text| text.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        let proposed_json: Value = serde_json::from_str(&proposed_text)?;
        let candidate_id = proposed_json["id"].as_str().expect("candidate id");

        let classified = client
            .call_tool(
                CallToolRequestParams::new("classify_memory_candidate").with_arguments(
                    serde_json::json!({"id": candidate_id})
                        .as_object()
                        .expect("classification arguments")
                        .clone(),
                ),
            )
            .await?;
        assert_ne!(classified.is_error, Some(true), "classification failed");
        let classified_text = classified
            .content
            .iter()
            .filter_map(|content| content.as_text())
            .map(|text| text.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        let classification: Value = serde_json::from_str(&classified_text)?;
        assert_eq!(classification["candidate_id"], candidate_id);
        assert_eq!(classification["classification"], "temporary-working");
        assert_eq!(classification["compared_memory_count"], 0);

        let reflected = client
            .call_tool(
                CallToolRequestParams::new("reflect_memory").with_arguments(
                    serde_json::json!({
                        "objective": "summarize active work memory",
                        "project": "work"
                    })
                    .as_object()
                    .expect("reflection arguments")
                    .clone(),
                ),
            )
            .await?;
        assert_ne!(reflected.is_error, Some(true), "reflection failed");
        let reflected_text = reflected
            .content
            .iter()
            .filter_map(|content| content.as_text())
            .map(|text| text.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        let reflection: Value = serde_json::from_str(&reflected_text)?;
        assert_eq!(reflection["status"], "completed");
        assert_eq!(reflection["metrics"]["canonical_memory_mutated"], false);

        let derived = client
            .call_tool(
                CallToolRequestParams::new("inspect_memory_representations").with_arguments(
                    serde_json::json!({"project": "work", "limit": 20})
                        .as_object()
                        .expect("derived arguments")
                        .clone(),
                ),
            )
            .await?;
        assert_ne!(derived.is_error, Some(true), "derived inspection failed");
        let derived_text = derived
            .content
            .iter()
            .filter_map(|content| content.as_text())
            .map(|text| text.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        let derived_json: Value = serde_json::from_str(&derived_text)?;
        assert_eq!(
            derived_json["contract_version"],
            "cortana.memory-derived.v1"
        );
        assert_eq!(derived_json["canonical_memory_mutated"], false);

        let after: CallToolResult = client
            .call_tool(CallToolRequestParams::new("brain_status"))
            .await?;
        let after_text = after
            .content
            .iter()
            .filter_map(|content| content.as_text())
            .map(|text| text.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        let after_stats: Value = serde_json::from_str(&after_text)?;
        assert_eq!(after_stats["memory_revision"], stats["memory_revision"]);

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
