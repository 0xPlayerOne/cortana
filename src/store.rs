use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use rusqlite::types::Type;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params, params_from_iter};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::auth::acl_allows;
use crate::memory::{self, MemoryInput, MemoryRecord, MemorySearchResult, MemoryStats};
use crate::model::{Document, StoredChunk};

const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const SYNC_RUNS_PER_SOURCE: usize = 100;

struct ExistingMemory {
    id: String,
    kind: String,
    project: String,
    title: String,
    content: String,
    source: String,
    source_id: String,
    status: String,
    acl: Vec<String>,
    confidence: f64,
    importance: f64,
    provenance_json: String,
    valid_until: Option<String>,
    supersedes_id: Option<String>,
}

fn bump_corpus_revision(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    transaction.execute(
        "UPDATE meta SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT)
         WHERE key='corpus_revision'",
        [],
    )?;
    Ok(())
}

fn bump_memory_revision(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    transaction.execute(
        "UPDATE meta SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT)
         WHERE key='memory_revision'",
        [],
    )?;
    Ok(())
}

#[derive(Clone)]
pub struct Store {
    connection: Arc<Mutex<Connection>>,
    read_connection: Arc<Mutex<Connection>>,
    memory_max_active: Arc<AtomicUsize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StoreStats {
    pub documents: i64,
    pub chunks: i64,
    pub embedding_fingerprint: Option<String>,
    pub embedding_cache_entries: i64,
    pub embedding_cache_hits: i64,
    pub query_cache_entries: i64,
    pub query_cache_hits: i64,
    pub sources: Vec<SourceStats>,
    pub sync_runs: Vec<SourceSyncStats>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PublicAclSummary {
    pub project: String,
    pub documents: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceStats {
    pub source: String,
    pub project: String,
    pub documents: i64,
    pub chunks: i64,
    pub latest_updated_at: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceSyncStats {
    pub source: String,
    pub project: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub documents: Option<i64>,
    pub bytes: Option<i64>,
    pub deleted: Option<i64>,
    pub budget_documents: i64,
    pub budget_bytes: i64,
    pub budget_seconds: i64,
}

#[derive(Debug, Serialize)]
pub struct AuditEvent {
    pub timestamp: String,
    pub principal: String,
    pub action: String,
    pub project: Option<String>,
    pub source: Option<String>,
    pub outcome: String,
    pub result_count: Option<i64>,
    pub latency_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct DocumentSummary {
    pub id: String,
    pub source: String,
    pub source_id: String,
    pub title: String,
    pub uri: Option<String>,
    pub updated_at: String,
    pub project: String,
    pub chunk_count: usize,
    pub content_chars: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct DocumentReference {
    pub id: String,
    pub source: String,
    pub source_id: String,
    pub title: String,
    pub uri: Option<String>,
    pub updated_at: String,
    pub project: String,
}

#[derive(Debug, Serialize)]
pub struct DocumentDetail {
    #[serde(flatten)]
    pub summary: DocumentSummary,
    pub content: String,
    pub metadata: Value,
    pub acl: Vec<String>,
    pub backlinks: Vec<DocumentReference>,
    pub surrounding: Vec<DocumentReference>,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct DocumentPage {
    pub documents: Vec<DocumentSummary>,
    pub has_more: bool,
}

#[derive(Clone, Debug)]
pub struct DocumentCursor {
    pub updated_at: String,
    pub id: String,
}

#[derive(Clone, Copy, Debug)]
pub enum SyncRunStatus {
    Succeeded,
    Failed,
    Cancelled,
    BudgetExceeded,
}

impl SyncRunStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::BudgetExceeded => "budget_exceeded",
        }
    }
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        reject_database_symlinks(path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(DATABASE_BUSY_TIMEOUT)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS documents(
               id TEXT PRIMARY KEY, source TEXT NOT NULL, source_id TEXT NOT NULL,
               title TEXT NOT NULL, uri TEXT, content_hash TEXT NOT NULL,
               updated_at TEXT NOT NULL, project TEXT NOT NULL, acl_json TEXT NOT NULL,
               metadata_json TEXT NOT NULL, content TEXT NOT NULL DEFAULT '',
               UNIQUE(source, source_id));
             CREATE TABLE IF NOT EXISTS document_links(
               document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
               target TEXT NOT NULL,
               PRIMARY KEY(document_id,target));
             CREATE TABLE IF NOT EXISTS chunks(
               id TEXT PRIMARY KEY, document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
               ordinal INTEGER NOT NULL, content TEXT NOT NULL, embedding_json TEXT NOT NULL,
               embedding_blob BLOB);
             CREATE TABLE IF NOT EXISTS embedding_cache(
               fingerprint TEXT NOT NULL, content_hash TEXT NOT NULL,
               embedding_json TEXT NOT NULL, embedding_blob BLOB, hits INTEGER NOT NULL DEFAULT 0,
               created_at TEXT NOT NULL, last_used_at TEXT NOT NULL,
               PRIMARY KEY(fingerprint,content_hash));
             CREATE TABLE IF NOT EXISTS sync_runs(
               id TEXT PRIMARY KEY, source TEXT NOT NULL, project TEXT NOT NULL,
               status TEXT NOT NULL, started_at TEXT NOT NULL, completed_at TEXT,
               documents INTEGER, bytes INTEGER, deleted INTEGER,
               budget_documents INTEGER NOT NULL, budget_bytes INTEGER NOT NULL,
               budget_seconds INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS query_cache(
               cache_key TEXT PRIMARY KEY, response_json TEXT NOT NULL,
               created_at TEXT NOT NULL, last_used_at TEXT NOT NULL,
               hits INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS audit_events(
               id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp TEXT NOT NULL,
               principal TEXT NOT NULL, action TEXT NOT NULL, project TEXT, source TEXT,
               outcome TEXT NOT NULL, result_count INTEGER, latency_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS memories(
               id TEXT PRIMARY KEY,
               kind TEXT NOT NULL,
               project TEXT NOT NULL,
               title TEXT NOT NULL,
               content TEXT NOT NULL,
               source TEXT NOT NULL,
               source_id TEXT NOT NULL,
               dedupe_key TEXT UNIQUE,
               confidence REAL NOT NULL,
               importance REAL NOT NULL,
               status TEXT NOT NULL,
               acl_json TEXT NOT NULL,
               provenance_json TEXT NOT NULL,
               observed_at TEXT NOT NULL,
               valid_from TEXT NOT NULL,
               valid_until TEXT,
               supersedes_id TEXT,
               created_at TEXT NOT NULL,
               updated_at TEXT NOT NULL);
             CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
               memory_id UNINDEXED, title, content, tokenize='unicode61');
             CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
               chunk_id UNINDEXED, title, content, tokenize='unicode61');
             CREATE INDEX IF NOT EXISTS idx_documents_scope ON documents(project, source);
             CREATE INDEX IF NOT EXISTS idx_documents_browse
               ON documents(updated_at DESC,id DESC);
             CREATE INDEX IF NOT EXISTS idx_document_links_target
               ON document_links(target);
             CREATE INDEX IF NOT EXISTS idx_chunks_document_ordinal
               ON chunks(document_id,ordinal);
             CREATE INDEX IF NOT EXISTS idx_sync_runs_source
               ON sync_runs(source,project,started_at DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_events_timestamp
               ON audit_events(timestamp DESC);
             CREATE INDEX IF NOT EXISTS idx_memories_scope
               ON memories(project,kind,status,updated_at DESC);
             CREATE INDEX IF NOT EXISTS idx_memories_status
               ON memories(status,updated_at DESC);",
        )?;
        connection.execute(
            "INSERT OR IGNORE INTO meta(key,value) VALUES('corpus_revision','0')",
            [],
        )?;
        connection.execute(
            "INSERT OR IGNORE INTO meta(key,value) VALUES('memory_revision','0')",
            [],
        )?;
        ensure_document_content_column(&connection)?;
        backfill_document_links(&mut connection)?;
        migrate_embedding_blobs(&mut connection)?;
        secure_database_files(path)?;
        let read_connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        read_connection.busy_timeout(DATABASE_BUSY_TIMEOUT)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            read_connection: Arc::new(Mutex::new(read_connection)),
            memory_max_active: Arc::new(AtomicUsize::new(memory::DEFAULT_MEMORY_MAX_ACTIVE)),
        })
    }

    /// Configure the active-memory ceiling on this process-wide store handle.
    /// The setting is shared by all clones used by HTTP, MCP, and CLI paths.
    pub fn configure_memory_limit(&self, max_active: usize) -> Result<()> {
        anyhow::ensure!(
            (1..=1_000_000).contains(&max_active),
            "memory max_active must be between 1 and 1000000"
        );
        self.memory_max_active
            .store(max_active, AtomicOrdering::Release);
        Ok(())
    }

    pub fn ensure_fingerprint(&self, fingerprint: &str) -> Result<()> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let current: Option<String> = connection
            .query_row(
                "SELECT value FROM meta WHERE key='embedding_fingerprint'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(index_fingerprint) = current.as_deref().filter(|value| *value != fingerprint) {
            bail!(
                "embedding model differs from this index (index: {index_fingerprint}; configured: {fingerprint}); rebuild into a new generation"
            );
        }
        connection.execute(
            "INSERT OR IGNORE INTO meta(key,value) VALUES('embedding_fingerprint',?1)",
            [fingerprint],
        )?;
        Ok(())
    }

    /// Adopt a reviewed embedding generation without touching indexed documents.
    ///
    /// This is intentionally stricter than `ensure_fingerprint`: callers must
    /// name the exact generation currently stored in the index. The operation
    /// invalidates derived caches because their vectors were produced under the
    /// old generation, while leaving documents and their stored vectors in
    /// place for an explicit operator-approved migration.
    pub fn migrate_embedding_fingerprint(&self, from: &str, to: &str) -> Result<()> {
        anyhow::ensure!(
            !from.trim().is_empty(),
            "source embedding fingerprint is empty"
        );
        anyhow::ensure!(
            !to.trim().is_empty(),
            "target embedding fingerprint is empty"
        );
        anyhow::ensure!(
            from != to,
            "source and target embedding generations are identical"
        );

        let mut connection = self.connection.lock().expect("store lock poisoned");
        let transaction = connection.transaction()?;
        let current: Option<String> = transaction
            .query_row(
                "SELECT value FROM meta WHERE key='embedding_fingerprint'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(current) = current else {
            bail!(
                "the index has no embedding generation; initialize it with the configured provider first"
            );
        };
        anyhow::ensure!(
            current == from,
            "embedding generation changed while preparing migration (expected: {from}; actual: {current})"
        );
        let changed = transaction.execute(
            "UPDATE meta SET value=?1 WHERE key='embedding_fingerprint' AND value=?2",
            params![to, from],
        )?;
        anyhow::ensure!(
            changed == 1,
            "embedding generation migration did not update the index"
        );
        transaction.execute("DELETE FROM embedding_cache", [])?;
        transaction.execute("DELETE FROM query_cache", [])?;
        transaction.commit()?;
        Ok(())
    }

    /// Prepare an atomic, full-corpus embedding rebuild.
    ///
    /// New vectors are staged separately from the live chunk vectors. The
    /// active generation is not changed until `commit_embedding_rebuild`
    /// verifies that every chunk has a replacement vector, so an interrupted
    /// provider call leaves the old index usable.
    pub fn begin_embedding_rebuild(&self, from: &str, to: &str) -> Result<usize> {
        anyhow::ensure!(
            !from.trim().is_empty(),
            "source embedding fingerprint is empty"
        );
        anyhow::ensure!(
            !to.trim().is_empty(),
            "target embedding fingerprint is empty"
        );
        anyhow::ensure!(
            from != to,
            "source and target embedding generations are identical"
        );

        let mut connection = self.connection.lock().expect("store lock poisoned");
        let transaction = connection.transaction()?;
        let current: Option<String> = transaction
            .query_row(
                "SELECT value FROM meta WHERE key='embedding_fingerprint'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(current) = current else {
            bail!(
                "the index has no embedding generation; initialize it with the configured provider first"
            );
        };
        anyhow::ensure!(
            current == from,
            "embedding generation changed while preparing rebuild (expected: {from}; actual: {current})"
        );
        let chunks: i64 =
            transaction.query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))?;
        transaction.execute(
            "CREATE TABLE IF NOT EXISTS embedding_rebuild(
               chunk_id TEXT PRIMARY KEY,
               embedding_blob BLOB NOT NULL
             )",
            [],
        )?;
        transaction.execute("DELETE FROM embedding_rebuild", [])?;
        transaction.commit()?;
        Ok(usize::try_from(chunks).unwrap_or(usize::MAX))
    }

    /// Return a stable, bounded page of chunk text for an embedding rebuild.
    pub fn embedding_rebuild_chunks(
        &self,
        after_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(
            "SELECT id,content FROM chunks
             WHERE (?1 IS NULL OR id>?1)
             ORDER BY id LIMIT ?2",
        )?;
        let rows = statement.query_map(
            params![after_id, i64::try_from(limit.max(1)).unwrap_or(i64::MAX)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Stage replacement vectors without changing the live index.
    pub fn stage_embedding_rebuild(&self, vectors: &[(String, Vec<f32>)]) -> Result<()> {
        let mut connection = self.connection.lock().expect("store lock poisoned");
        let transaction = connection.transaction()?;
        for (chunk_id, embedding) in vectors {
            anyhow::ensure!(
                !embedding.is_empty(),
                "embedding rebuild produced an empty vector"
            );
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM chunks WHERE id=?1)",
                [chunk_id],
                |row| row.get(0),
            )?;
            anyhow::ensure!(exists, "embedding rebuild referenced an unknown chunk");
            transaction.execute(
                "INSERT INTO embedding_rebuild(chunk_id,embedding_blob) VALUES(?1,?2)
                 ON CONFLICT(chunk_id) DO UPDATE SET embedding_blob=excluded.embedding_blob",
                params![chunk_id, encode_embedding(embedding)],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Atomically install staged vectors and adopt the target generation.
    pub fn commit_embedding_rebuild(&self, from: &str, to: &str) -> Result<usize> {
        let mut connection = self.connection.lock().expect("store lock poisoned");
        let transaction = connection.transaction()?;
        let current: Option<String> = transaction
            .query_row(
                "SELECT value FROM meta WHERE key='embedding_fingerprint'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        anyhow::ensure!(
            current.as_deref() == Some(from),
            "embedding generation changed while committing rebuild"
        );
        let total: i64 =
            transaction.query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))?;
        let staged: i64 =
            transaction.query_row("SELECT COUNT(*) FROM embedding_rebuild", [], |row| {
                row.get(0)
            })?;
        anyhow::ensure!(
            staged == total,
            "embedding rebuild is incomplete: staged {staged} of {total} chunks"
        );
        let changed = transaction.execute(
            "UPDATE chunks SET embedding_json='[]',embedding_blob=(
                 SELECT embedding_blob FROM embedding_rebuild
                 WHERE embedding_rebuild.chunk_id=chunks.id
             )",
            [],
        )?;
        anyhow::ensure!(
            i64::try_from(changed).unwrap_or(i64::MAX) == total,
            "embedding rebuild updated an unexpected number of chunks"
        );
        let generation_changed = transaction.execute(
            "UPDATE meta SET value=?1 WHERE key='embedding_fingerprint' AND value=?2",
            params![to, from],
        )?;
        anyhow::ensure!(
            generation_changed == 1,
            "embedding generation rebuild did not update the index"
        );
        transaction.execute("DELETE FROM embedding_cache", [])?;
        transaction.execute("DELETE FROM query_cache", [])?;
        bump_corpus_revision(&transaction)?;
        transaction.execute("DROP TABLE embedding_rebuild", [])?;
        transaction.commit()?;
        Ok(usize::try_from(total).unwrap_or(usize::MAX))
    }

    /// Remove a staged rebuild after a provider or validation failure.
    pub fn discard_embedding_rebuild(&self) -> Result<()> {
        let connection = self.connection.lock().expect("store lock poisoned");
        connection.execute("DROP TABLE IF EXISTS embedding_rebuild", [])?;
        Ok(())
    }

    pub fn begin_sync(
        &self,
        source: &str,
        project: &str,
        budget_documents: usize,
        budget_bytes: u64,
        budget_seconds: u64,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut connection = self.connection.lock().expect("store lock poisoned");
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO sync_runs(
               id,source,project,status,started_at,
               budget_documents,budget_bytes,budget_seconds)
             VALUES(?1,?2,?3,'running',?4,?5,?6,?7)",
            params![
                id,
                source,
                project,
                Utc::now().to_rfc3339(),
                i64::try_from(budget_documents).unwrap_or(i64::MAX),
                i64::try_from(budget_bytes).unwrap_or(i64::MAX),
                i64::try_from(budget_seconds).unwrap_or(i64::MAX),
            ],
        )?;
        transaction.execute(
            "DELETE FROM sync_runs
             WHERE id IN (
               SELECT id FROM sync_runs
               WHERE source=?1 AND project=?2
               ORDER BY started_at DESC,rowid DESC
               LIMIT -1 OFFSET ?3
             )",
            params![
                source,
                project,
                i64::try_from(SYNC_RUNS_PER_SOURCE).unwrap_or(i64::MAX)
            ],
        )?;
        transaction.commit()?;
        Ok(id)
    }

    pub fn finish_sync(
        &self,
        id: &str,
        status: SyncRunStatus,
        documents: Option<usize>,
        bytes: Option<u64>,
        deleted: Option<usize>,
    ) -> Result<()> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let changed = connection.execute(
            "UPDATE sync_runs
             SET status=?2,completed_at=?3,documents=?4,bytes=?5,deleted=?6
             WHERE id=?1 AND status='running'",
            params![
                id,
                status.as_str(),
                Utc::now().to_rfc3339(),
                documents.map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
                bytes.map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
                deleted.map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
            ],
        )?;
        anyhow::ensure!(changed == 1, "sync run is missing or already completed");
        Ok(())
    }

    /// Recover sync runs left `running` by an interrupted process.
    ///
    /// Marks every still-`running` run as `cancelled` with a completion
    /// timestamp. Completed and failed runs are left untouched, including any
    /// recorded outcome counters and completion timestamps.
    /// Callers must hold the global sync lock first so no live sync can own the
    /// affected records. This is metadata-only: it never touches document,
    /// chunk, or index data, and it does not delete run history or alter the
    /// per-source retention bound. Returns the number of recovered runs.
    pub fn recover_interrupted_syncs(&self) -> Result<usize> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let changed = connection.execute(
            "UPDATE sync_runs
             SET status=?1,completed_at=?2
             WHERE status='running'",
            params![SyncRunStatus::Cancelled.as_str(), Utc::now().to_rfc3339()],
        )?;
        Ok(changed)
    }

    pub fn upsert(&self, document: &Document, chunks: &[(String, Vec<f32>)]) -> Result<bool> {
        let id = stable_id(&document.source, &document.source_id);
        let hash = document_hash(document)?;
        let mut connection = self.connection.lock().expect("store lock poisoned");
        let previous: Option<(String, bool)> = connection
            .query_row(
                "SELECT content_hash,length(content)>0 FROM documents WHERE id=?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if previous
            .as_ref()
            .is_some_and(|(previous_hash, has_content)| previous_hash == &hash && *has_content)
        {
            return Ok(false);
        }
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM chunks_fts WHERE chunk_id IN (SELECT id FROM chunks WHERE document_id=?1)",
            [&id],
        )?;
        transaction.execute("DELETE FROM chunks WHERE document_id=?1", [&id])?;
        transaction.execute(
            "INSERT INTO documents(id,source,source_id,title,uri,content_hash,updated_at,project,acl_json,metadata_json,content)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(id) DO UPDATE SET title=excluded.title,uri=excluded.uri,
             content_hash=excluded.content_hash,updated_at=excluded.updated_at,project=excluded.project,
             acl_json=excluded.acl_json,metadata_json=excluded.metadata_json,content=excluded.content",
            params![id, document.source, document.source_id, document.title, document.uri, hash,
                document.updated_at.to_rfc3339(), document.project,
                serde_json::to_string(&document.acl)?, document.metadata.to_string(),
                document.content],
        )?;
        transaction.execute("DELETE FROM document_links WHERE document_id=?1", [&id])?;
        for target in metadata_reference_strings(&document.metadata) {
            transaction.execute(
                "INSERT OR IGNORE INTO document_links(document_id,target) VALUES(?1,?2)",
                params![id, target],
            )?;
        }
        for (ordinal, (content, embedding)) in chunks.iter().enumerate() {
            let chunk_id = format!("{id}:{ordinal}");
            transaction.execute(
                "INSERT INTO chunks(
                   id,document_id,ordinal,content,embedding_json,embedding_blob
                 ) VALUES(?1,?2,?3,?4,'[]',?5)",
                params![
                    chunk_id,
                    id,
                    ordinal as i64,
                    content,
                    encode_embedding(embedding)
                ],
            )?;
            transaction.execute(
                "INSERT INTO chunks_fts(chunk_id,title,content) VALUES(?1,?2,?3)",
                params![chunk_id, document.title, content],
            )?;
        }
        bump_corpus_revision(&transaction)?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn needs_update(&self, document: &Document) -> Result<bool> {
        let id = stable_id(&document.source, &document.source_id);
        let hash = document_hash(document)?;
        let connection = self.connection.lock().expect("store lock poisoned");
        let previous: Option<(String, bool)> = connection
            .query_row(
                "SELECT content_hash,length(content)>0 FROM documents WHERE id=?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(!previous
            .as_ref()
            .is_some_and(|(previous_hash, has_content)| previous_hash == &hash && *has_content))
    }

    pub fn refresh_timestamp(&self, document: &Document) -> Result<()> {
        let id = stable_id(&document.source, &document.source_id);
        let mut connection = self.connection.lock().expect("store lock poisoned");
        let transaction = connection.transaction()?;
        let updated_at = document.updated_at.to_rfc3339();
        let changed = transaction.execute(
            "UPDATE documents SET updated_at=?2 WHERE id=?1 AND updated_at<>?2",
            params![id, updated_at],
        )?;
        if changed > 0 {
            bump_corpus_revision(&transaction)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn reconcile(
        &self,
        source: &str,
        project: &str,
        seen_source_ids: &[String],
    ) -> Result<usize> {
        let mut connection = self.connection.lock().expect("store lock poisoned");
        let transaction = connection.transaction()?;
        transaction.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS reconcile_seen(
               source_id TEXT PRIMARY KEY
             ) WITHOUT ROWID;
             DELETE FROM reconcile_seen;",
        )?;
        {
            let mut insert = transaction
                .prepare("INSERT OR IGNORE INTO reconcile_seen(source_id) VALUES(?1)")?;
            for source_id in seen_source_ids {
                insert.execute([source_id])?;
            }
        }
        let stale = {
            let mut statement = transaction.prepare(
                "SELECT d.id FROM documents d
                 WHERE d.source=?1 AND d.project=?2
                   AND NOT EXISTS(
                     SELECT 1 FROM reconcile_seen s WHERE s.source_id=d.source_id
                   )",
            )?;
            statement
                .query_map(params![source, project], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for id in &stale {
            transaction.execute(
                "DELETE FROM chunks_fts WHERE chunk_id IN
                 (SELECT id FROM chunks WHERE document_id=?1)",
                [id],
            )?;
            transaction.execute("DELETE FROM documents WHERE id=?1", [id])?;
        }
        if !stale.is_empty() {
            bump_corpus_revision(&transaction)?;
        }
        transaction.commit()?;
        Ok(stale.len())
    }

    pub fn corpus_revision(&self) -> Result<u64> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let value: String = connection.query_row(
            "SELECT value FROM meta WHERE key='corpus_revision'",
            [],
            |row| row.get(0),
        )?;
        value.parse().context("invalid corpus revision")
    }

    pub fn memory_revision(&self) -> Result<u64> {
        let connection = self.read_connection.lock().expect("store lock poisoned");
        let value: String = connection.query_row(
            "SELECT value FROM meta WHERE key='memory_revision'",
            [],
            |row| row.get(0),
        )?;
        value.parse().context("invalid memory revision")
    }

    pub fn cached_query(&self, cache_key: &str, ttl_seconds: u64) -> Result<Option<String>> {
        if ttl_seconds == 0 {
            return Ok(None);
        }
        let connection = self.connection.lock().expect("store lock poisoned");
        let value: Option<(String, String)> = connection
            .query_row(
                "SELECT response_json,created_at FROM query_cache WHERE cache_key=?1",
                [cache_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((response, created_at)) = value else {
            return Ok(None);
        };
        let created_at = match DateTime::parse_from_rfc3339(&created_at) {
            Ok(created_at) => created_at.with_timezone(&Utc),
            Err(_) => {
                let _ = optional_write(&connection, || {
                    connection.execute("DELETE FROM query_cache WHERE cache_key=?1", [cache_key])
                });
                return Ok(None);
            }
        };
        if (Utc::now() - created_at).num_seconds() > ttl_seconds as i64 {
            optional_write(&connection, || {
                connection.execute("DELETE FROM query_cache WHERE cache_key=?1", [cache_key])
            })?;
            return Ok(None);
        }
        optional_write(&connection, || {
            connection.execute(
                "UPDATE query_cache SET hits=hits+1,last_used_at=?2 WHERE cache_key=?1",
                params![cache_key, Utc::now().to_rfc3339()],
            )
        })?;
        Ok(Some(response))
    }

    /// Remove one answer-cache row after the caller detects that its payload
    /// no longer matches the current response contract. Cache cleanup is
    /// best-effort, just like hit counters, so a busy writer must not turn a
    /// valid retrieval into an error.
    pub fn invalidate_cached_query(&self, cache_key: &str) -> Result<()> {
        let connection = self.connection.lock().expect("store lock poisoned");
        optional_write(&connection, || {
            connection.execute("DELETE FROM query_cache WHERE cache_key=?1", [cache_key])
        })?;
        Ok(())
    }

    pub fn cache_query(&self, cache_key: &str, response: &str, max_entries: usize) -> Result<()> {
        if max_entries == 0 {
            return Ok(());
        }
        let now = Utc::now().to_rfc3339();
        let connection = self.connection.lock().expect("store lock poisoned");
        connection.execute(
            "INSERT INTO query_cache(cache_key,response_json,created_at,last_used_at,hits)
             VALUES(?1,?2,?3,?3,0)
             ON CONFLICT(cache_key) DO UPDATE SET
               response_json=excluded.response_json,created_at=excluded.created_at,
               last_used_at=excluded.last_used_at",
            params![cache_key, response, now],
        )?;
        connection.execute(
            "DELETE FROM query_cache WHERE cache_key IN (
               SELECT cache_key FROM query_cache
               ORDER BY last_used_at DESC
               LIMIT -1 OFFSET ?1
             )",
            [i64::try_from(max_entries).unwrap_or(i64::MAX)],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_audit(
        &self,
        principal: &str,
        action: &str,
        project: Option<&str>,
        source: Option<&str>,
        outcome: &str,
        result_count: Option<usize>,
        latency_ms: u64,
        max_events: usize,
    ) -> Result<bool> {
        if max_events == 0 {
            return Ok(false);
        }
        let connection = self.connection.lock().expect("store lock poisoned");
        let Some(_) = optional_write(&connection, || {
            connection.execute(
                "INSERT INTO audit_events(
                   timestamp,principal,action,project,source,outcome,result_count,latency_ms)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    Utc::now().to_rfc3339(),
                    principal,
                    action,
                    project,
                    source,
                    outcome,
                    result_count.map(|count| i64::try_from(count).unwrap_or(i64::MAX)),
                    i64::try_from(latency_ms).unwrap_or(i64::MAX),
                ],
            )
        })?
        else {
            return Ok(false);
        };
        optional_write(&connection, || {
            connection.execute(
                "DELETE FROM audit_events WHERE id IN (
                   SELECT id FROM audit_events ORDER BY id DESC LIMIT -1 OFFSET ?1
                 )",
                [i64::try_from(max_events).unwrap_or(i64::MAX)],
            )
        })?;
        Ok(true)
    }

    pub fn audit_events(&self, limit: usize) -> Result<Vec<AuditEvent>> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(
            "SELECT timestamp,principal,action,project,source,outcome,result_count,latency_ms
             FROM audit_events ORDER BY id DESC LIMIT ?1",
        )?;
        statement
            .query_map([i64::try_from(limit.min(500)).unwrap_or(500)], |row| {
                Ok(AuditEvent {
                    timestamp: row.get(0)?,
                    principal: row.get(1)?,
                    action: row.get(2)?,
                    project: row.get(3)?,
                    source: row.get(4)?,
                    outcome: row.get(5)?,
                    result_count: row.get(6)?,
                    latency_ms: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Return the retained metadata-only audit trail for an operator export.
    ///
    /// The HTTP endpoint intentionally caps responses at 500 rows. Exports
    /// need to preserve the configured retention window instead, so the CLI
    /// applies its own explicit upper bound before calling this method.
    pub fn audit_events_for_export(&self, limit: usize) -> Result<Vec<AuditEvent>> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(
            "SELECT timestamp,principal,action,project,source,outcome,result_count,latency_ms
             FROM audit_events ORDER BY id DESC LIMIT ?1",
        )?;
        let mut events = statement
            .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
                Ok(AuditEvent {
                    timestamp: row.get(0)?,
                    principal: row.get(1)?,
                    action: row.get(2)?,
                    project: row.get(3)?,
                    source: row.get(4)?,
                    outcome: row.get(5)?,
                    result_count: row.get(6)?,
                    latency_ms: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // Select the newest retained events like the interactive endpoint,
        // then emit them oldest-first so exports replay chronologically.
        events.reverse();
        Ok(events)
    }

    /// Persist an explicit agent memory in the canonical SQLite store.
    ///
    /// Memory writes are idempotent when a caller supplies `dedupe_key`; this
    /// lets an agent retry a tool call without accumulating duplicate facts.
    /// Superseding is transactional so the previous memory cannot remain
    /// active after its replacement is committed.
    pub fn remember(&self, input: &MemoryInput) -> Result<MemoryRecord> {
        self.remember_scoped(input, &["*".into()], true)
    }

    /// Persist a memory while enforcing the caller's ACL inside the same
    /// transaction as dedupe and supersession.  The pre-existing record must
    /// be visible before a scoped agent can replace it or supersede it; doing
    /// this in the store avoids a check-then-write race in HTTP and MCP.
    pub fn remember_scoped(
        &self,
        input: &MemoryInput,
        principal_acl: &[String],
        owner: bool,
    ) -> Result<MemoryRecord> {
        let (kind, acl, provenance_json, valid_until) = memory::validate_input(input)?;
        let now = memory::now();
        let mut connection = self.connection.lock().expect("store lock poisoned");
        let transaction = connection.transaction()?;
        let max_active = self.memory_max_active.load(AtomicOrdering::Acquire);
        let existing: Option<ExistingMemory> = if let Some(key) = input.dedupe_key.as_deref() {
            transaction
                .query_row(
                    "SELECT id,kind,project,title,content,source,source_id,status,
                            acl_json,confidence,importance,provenance_json,valid_until,
                            supersedes_id
                     FROM memories WHERE dedupe_key=?1",
                    [key],
                    |row| {
                        let acl_json: String = row.get(8)?;
                        let acl = serde_json::from_str(&acl_json).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                8,
                                Type::Text,
                                Box::new(error),
                            )
                        })?;
                        Ok(ExistingMemory {
                            id: row.get(0)?,
                            kind: row.get(1)?,
                            project: row.get(2)?,
                            title: row.get(3)?,
                            content: row.get(4)?,
                            source: row.get(5)?,
                            source_id: row.get(6)?,
                            status: row.get(7)?,
                            acl,
                            confidence: row.get(9)?,
                            importance: row.get(10)?,
                            provenance_json: row.get(11)?,
                            valid_until: row.get(12)?,
                            supersedes_id: row.get(13)?,
                        })
                    },
                )
                .optional()?
        } else {
            None
        };
        if let Some(existing) = &existing {
            anyhow::ensure!(
                owner || acl_allows(&existing.acl, principal_acl),
                "memory dedupe key is outside principal visibility"
            );
        }
        let id = existing
            .as_ref()
            .map(|memory| memory.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let source_id = if input.source_id.trim().is_empty() {
            id.clone()
        } else {
            input.source_id.clone()
        };
        // A retry with the same dedupe key and identical normalized payload is
        // a true no-op. Avoiding a write keeps memory_revision stable, which in
        // turn preserves answer-cache hits for idempotent agent retries.
        if let Some(existing) = &existing {
            if existing.status == "active"
                && existing.kind == kind.as_str()
                && existing.project == input.project
                && existing.title == input.title
                && existing.content == input.content
                && existing.source == input.source
                && existing.source_id == source_id
                && existing.confidence == f64::from(input.confidence)
                && existing.importance == f64::from(input.importance)
                && existing.acl == acl
                && existing.provenance_json == provenance_json
                && existing.valid_until == valid_until
                && existing.supersedes_id == input.supersedes_id
            {
                transaction.commit()?;
                return self
                    .memory(&existing.id)?
                    .ok_or_else(|| anyhow::anyhow!("memory disappeared during idempotent retry"));
            }
        }
        let active_count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM memories
             WHERE status='active' AND (valid_until IS NULL OR julianday(valid_until)>julianday(?1))",
            [now.as_str()],
            |row| row.get(0),
        )?;
        let replaces_active = existing.as_ref().is_some_and(|memory| {
            memory.status == "active"
                && memory::valid_until_is_active(memory.valid_until.as_deref(), &now)
        });
        let supersession_target: Option<(Vec<String>, Option<String>)> =
            if let Some(previous_id) = input.supersedes_id.as_deref() {
                transaction
                    .query_row(
                        "SELECT acl_json,valid_until FROM memories
                         WHERE id=?1 AND status='active'",
                        [previous_id],
                        |row| {
                            let acl_json: String = row.get(0)?;
                            let acl = serde_json::from_str(&acl_json).map_err(|error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    0,
                                    Type::Text,
                                    Box::new(error),
                                )
                            })?;
                            Ok((acl, row.get(1)?))
                        },
                    )
                    .optional()?
            } else {
                None
            };
        let supersedes_active = supersession_target
            .as_ref()
            .is_some_and(|(_, valid_until)| {
                memory::valid_until_is_active(valid_until.as_deref(), &now)
            });
        anyhow::ensure!(
            replaces_active || supersedes_active || active_count < max_active as i64,
            "active memory limit reached ({max_active}); retract or supersede an existing memory before adding another"
        );
        if let Some(previous_id) = input.supersedes_id.as_deref() {
            if let Some((previous_acl, _)) = &supersession_target {
                anyhow::ensure!(
                    owner || acl_allows(previous_acl, principal_acl),
                    "memory supersession target is outside principal visibility"
                );
            }
            let changed = transaction.execute(
                "UPDATE memories SET status='superseded',valid_until=?2,updated_at=?2
                 WHERE id=?1 AND status='active'",
                params![previous_id, now],
            )?;
            anyhow::ensure!(
                changed == 1,
                "supersedes_id does not identify an active memory"
            );
        }
        transaction.execute(
            "INSERT INTO memories(
               id,kind,project,title,content,source,source_id,dedupe_key,
               confidence,importance,status,acl_json,provenance_json,
               observed_at,valid_from,valid_until,supersedes_id,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'active',?11,?12,?13,?13,?14,?15,?13,?13)
             ON CONFLICT(id) DO UPDATE SET
               kind=excluded.kind,project=excluded.project,title=excluded.title,
               content=excluded.content,source=excluded.source,source_id=excluded.source_id,
               dedupe_key=excluded.dedupe_key,confidence=excluded.confidence,
               importance=excluded.importance,status='active',acl_json=excluded.acl_json,
               provenance_json=excluded.provenance_json,observed_at=excluded.observed_at,
               valid_from=excluded.valid_from,valid_until=excluded.valid_until,
               supersedes_id=excluded.supersedes_id,updated_at=excluded.updated_at",
            params![
                id,
                kind.as_str(),
                input.project,
                input.title,
                input.content,
                input.source,
                source_id,
                input.dedupe_key,
                f64::from(input.confidence),
                f64::from(input.importance),
                serde_json::to_string(&acl)?,
                provenance_json,
                now,
                valid_until,
                input.supersedes_id,
            ],
        )?;
        transaction.execute("DELETE FROM memories_fts WHERE memory_id=?1", [&id])?;
        transaction.execute(
            "INSERT INTO memories_fts(memory_id,title,content) VALUES(?1,?2,?3)",
            params![id, input.title, input.content],
        )?;
        bump_memory_revision(&transaction)?;
        transaction.commit()?;
        self.memory(&id)?
            .ok_or_else(|| anyhow::anyhow!("memory disappeared after commit"))
    }

    /// Recall active memories using bounded SQLite FTS and an explicit ACL.
    /// Content is never returned for retracted or superseded memories.
    pub fn recall_memories(
        &self,
        query: &str,
        project: Option<&str>,
        kind: Option<&str>,
        limit: usize,
        principal_acl: &[String],
    ) -> Result<Vec<MemorySearchResult>> {
        let match_query = memory::fts_query(query)?;
        let fallback_query = memory::fts_query_or(query)?;
        if let Some(kind) = kind {
            memory::MemoryKind::parse(kind)?;
        }
        let connection = self.read_connection.lock().expect("store lock poisoned");
        let candidate_limit =
            i64::try_from(limit.clamp(1, memory::MAX_MEMORY_RECALL_LIMIT) * 4).unwrap_or(i64::MAX);
        let now = memory::now();
        let principal_acl_json = serde_json::to_string(principal_acl)?;
        let mut results = Vec::new();
        let mut seen = HashSet::new();
        for (index, query_variant) in [match_query, fallback_query].into_iter().enumerate() {
            if index > 0 && results.len() >= limit.clamp(1, memory::MAX_MEMORY_RECALL_LIMIT) {
                break;
            }
            let mut statement = connection.prepare(
                "SELECT m.id,m.kind,m.project,m.title,m.content,m.source,m.source_id,
                        m.dedupe_key,m.confidence,m.importance,m.status,m.acl_json,
                        m.provenance_json,m.observed_at,m.valid_from,m.valid_until,
                        m.supersedes_id,m.created_at,m.updated_at,bm25(memories_fts)
                 FROM memories_fts
                 JOIN memories m ON m.id=memories_fts.memory_id
                 WHERE memories_fts MATCH ?1 AND m.status='active'
                   AND (?2 IS NULL OR m.project=?2)
                   AND (?3 IS NULL OR m.kind=?3)
                   AND m.valid_from<=?4
                   AND (m.valid_until IS NULL OR julianday(m.valid_until)>julianday(?4))
                   AND json_valid(m.acl_json)
                   AND json_type(m.acl_json)='array'
                   AND NOT EXISTS (
                     SELECT 1 FROM json_each(m.acl_json) AS memory_acl
                     WHERE memory_acl.type<>'text'
                   )
                   AND (
                     json_array_length(m.acl_json)=0
                     OR EXISTS (
                       SELECT 1 FROM json_each(?5) AS principal_acl
                       WHERE principal_acl.type='text'
                         AND (
                           principal_acl.value='*'
                           OR EXISTS (
                             SELECT 1 FROM json_each(m.acl_json) AS memory_acl
                             WHERE memory_acl.type='text'
                               AND memory_acl.value=principal_acl.value
                           )
                         )
                     )
                   )
                 ORDER BY bm25(memories_fts),m.importance DESC,m.confidence DESC,m.updated_at DESC
                 LIMIT ?6",
            )?;
            let rows = statement.query_map(
                params![
                    query_variant,
                    project,
                    kind,
                    now,
                    principal_acl_json,
                    candidate_limit
                ],
                memory_from_row,
            )?;
            for row in rows {
                let memory = row?;
                if acl_allows(&memory.memory.acl, principal_acl)
                    && seen.insert(memory.memory.id.clone())
                {
                    results.push(memory);
                    if results.len() >= limit.clamp(1, memory::MAX_MEMORY_RECALL_LIMIT) {
                        return Ok(results);
                    }
                }
            }
        }
        Ok(results)
    }

    pub fn memory(&self, id: &str) -> Result<Option<MemoryRecord>> {
        let connection = self.read_connection.lock().expect("store lock poisoned");
        connection
            .query_row(
                "SELECT id,kind,project,title,content,source,source_id,dedupe_key,
                        confidence,importance,status,acl_json,provenance_json,
                        observed_at,valid_from,valid_until,supersedes_id,created_at,updated_at
                 FROM memories WHERE id=?1",
                [id],
                memory_record_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Redact memory content while retaining a tombstone for auditability.
    pub fn forget_memory(&self, id: &str) -> Result<bool> {
        self.forget_memory_scoped(id, &["*".into()], true)
    }

    /// Redact a memory only when the caller can still see it.  The ACL check
    /// and tombstone update share one write transaction so a concurrent
    /// replacement cannot turn a previously authorized read into an
    /// unauthorized mutation.
    pub fn forget_memory_scoped(
        &self,
        id: &str,
        principal_acl: &[String],
        owner: bool,
    ) -> Result<bool> {
        let now = memory::now();
        let mut connection = self.connection.lock().expect("store lock poisoned");
        let transaction = connection.transaction()?;
        let acl_json: Option<String> = transaction
            .query_row(
                "SELECT acl_json FROM memories WHERE id=?1 AND status='active'",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(acl_json) = acl_json else {
            transaction.commit()?;
            return Ok(false);
        };
        let acl: Vec<String> = serde_json::from_str(&acl_json)?;
        anyhow::ensure!(
            owner || acl_allows(&acl, principal_acl),
            "memory ACL denied"
        );
        let changed = transaction.execute(
            "UPDATE memories SET status='retracted',content='',provenance_json='{}',
             valid_until=COALESCE(valid_until,?2),updated_at=?2 WHERE id=?1 AND status='active'",
            params![id, now],
        )?;
        if changed == 1 {
            transaction.execute("DELETE FROM memories_fts WHERE memory_id=?1", [id])?;
            bump_memory_revision(&transaction)?;
        }
        transaction.commit()?;
        Ok(changed == 1)
    }

    pub fn memory_stats(&self) -> Result<MemoryStats> {
        let connection = self.read_connection.lock().expect("store lock poisoned");
        let now = memory::now();
        let (active, expired, retracted, superseded): (i64, i64, i64, i64) = connection.query_row(
            "SELECT
                   COALESCE(SUM(CASE WHEN status='active'
                     AND (valid_until IS NULL OR julianday(valid_until)>julianday(?1)) THEN 1 ELSE 0 END),0),
                   COALESCE(SUM(CASE WHEN status='active'
                     AND valid_until IS NOT NULL AND julianday(valid_until)<=julianday(?1) THEN 1 ELSE 0 END),0),
                   COALESCE(SUM(CASE WHEN status='retracted' THEN 1 ELSE 0 END),0),
                   COALESCE(SUM(CASE WHEN status='superseded' THEN 1 ELSE 0 END),0)
                 FROM memories",
            [now.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let mut stats = MemoryStats {
            active,
            expired,
            retracted,
            superseded,
            total: 0,
        };
        stats.total = stats.active + stats.expired + stats.retracted + stats.superseded;
        Ok(stats)
    }

    /// Export bounded memory records visible to the supplied ACL.  Tombstones
    /// are retained with their redacted content so a restore can preserve
    /// deletion history without exposing an out-of-scope record.
    pub fn export_memories(
        &self,
        project: Option<&str>,
        kind: Option<&str>,
        limit: usize,
        principal_acl: &[String],
    ) -> Result<Vec<MemoryRecord>> {
        if let Some(kind) = kind {
            memory::MemoryKind::parse(kind)?;
        }
        let limit = limit.clamp(1, memory::MAX_MEMORY_EXPORT_LIMIT);
        let connection = self.read_connection.lock().expect("store lock poisoned");
        let principal_acl_json = serde_json::to_string(principal_acl)?;
        let mut statement = connection.prepare(
            "SELECT id,kind,project,title,content,source,source_id,dedupe_key,
                    confidence,importance,status,acl_json,provenance_json,
                    observed_at,valid_from,valid_until,supersedes_id,created_at,updated_at
             FROM memories
               WHERE (?1 IS NULL OR project=?1)
               AND (?2 IS NULL OR kind=?2)
               AND json_valid(acl_json)
               AND json_type(acl_json)='array'
               AND NOT EXISTS (
                 SELECT 1 FROM json_each(acl_json) AS memory_acl
                 WHERE memory_acl.type<>'text'
               )
               AND (
                 json_array_length(acl_json)=0
                 OR EXISTS (
                   SELECT 1 FROM json_each(?3) AS principal_acl
                   WHERE principal_acl.type='text'
                     AND (
                       principal_acl.value='*'
                       OR EXISTS (
                         SELECT 1 FROM json_each(acl_json) AS memory_acl
                         WHERE memory_acl.type='text'
                           AND memory_acl.value=principal_acl.value
                       )
                     )
                 )
               )
             ORDER BY updated_at DESC,id DESC
             LIMIT ?4",
        )?;
        let rows = statement.query_map(
            params![
                project,
                kind,
                principal_acl_json,
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            memory_record_from_row,
        )?;
        let mut records = Vec::new();
        for row in rows {
            let record = row?;
            if acl_allows(&record.acl, principal_acl) {
                records.push(record);
                if records.len() >= limit {
                    break;
                }
            }
        }
        Ok(records)
    }

    pub fn list_documents_scoped(
        &self,
        project: Option<&str>,
        source: Option<&str>,
        query: Option<&str>,
        cursor: Option<&DocumentCursor>,
        limit: usize,
        principal_acl: &[String],
    ) -> Result<DocumentPage> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(
            "SELECT d.id,d.source,d.source_id,d.title,d.uri,d.updated_at,d.project,d.acl_json,
                    COUNT(c.id),
                    CASE WHEN length(d.content)>0 THEN length(d.content)
                         ELSE COALESCE(SUM(length(c.content)),0) END
             FROM documents d LEFT JOIN chunks c ON c.document_id=d.id
             WHERE (?1 IS NULL OR d.project=?1)
               AND (?2 IS NULL OR d.source=?2)
               AND (?3 IS NULL OR instr(lower(d.title),lower(?3))>0
                    OR instr(lower(d.source),lower(?3))>0
                    OR instr(lower(d.source_id),lower(?3))>0)
               AND (?4 IS NULL OR d.updated_at<?4 OR (d.updated_at=?4 AND d.id<?5))
             GROUP BY d.id
             ORDER BY d.updated_at DESC,d.id DESC
             LIMIT ?6",
        )?;
        let page_size = limit.clamp(1, 100);
        let mut scan_cursor = cursor.cloned();
        let mut documents = Vec::with_capacity(page_size.saturating_add(1));
        loop {
            let scan_limit = page_size.saturating_mul(4).max(64);
            let rows = statement
                .query_map(
                    params![
                        project,
                        source,
                        query,
                        scan_cursor.as_ref().map(|value| value.updated_at.as_str()),
                        scan_cursor.as_ref().map(|value| value.id.as_str()),
                        i64::try_from(scan_limit).unwrap_or(400),
                    ],
                    |row| {
                        let acl_json = row.get::<_, String>(7)?;
                        let acl =
                            serde_json::from_str::<Vec<String>>(&acl_json).map_err(|error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    7,
                                    Type::Text,
                                    Box::new(error),
                                )
                            })?;
                        let chunk_count =
                            usize::try_from(row.get::<_, i64>(8)?).unwrap_or(usize::MAX);
                        let content_chars =
                            usize::try_from(row.get::<_, i64>(9)?).unwrap_or(usize::MAX);
                        Ok((
                            DocumentSummary {
                                id: row.get(0)?,
                                source: row.get(1)?,
                                source_id: row.get(2)?,
                                title: row.get(3)?,
                                uri: row.get(4)?,
                                updated_at: row.get(5)?,
                                project: row.get(6)?,
                                chunk_count,
                                content_chars,
                            },
                            acl,
                        ))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let scanned = rows.len();
            for (summary, acl) in rows {
                scan_cursor = Some(DocumentCursor {
                    updated_at: summary.updated_at.clone(),
                    id: summary.id.clone(),
                });
                if acl_allows(&acl, principal_acl) {
                    documents.push(summary);
                    if documents.len() > page_size {
                        break;
                    }
                }
            }
            if documents.len() > page_size || scanned < scan_limit {
                break;
            }
        }
        let has_more = documents.len() > page_size;
        documents.truncate(page_size);
        Ok(DocumentPage {
            documents,
            has_more,
        })
    }

    pub fn document_scoped(
        &self,
        id: &str,
        principal_acl: &[String],
        max_content_bytes: usize,
    ) -> Result<Option<DocumentDetail>> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let record = connection
            .query_row(
                "SELECT d.source,d.source_id,d.title,d.uri,d.updated_at,d.project,d.acl_json,
                        d.metadata_json,
                        CAST(substr(CAST(d.content AS BLOB),1,?2) AS BLOB),
                        length(CAST(d.content AS BLOB)),COUNT(c.id),
                        CASE WHEN length(d.content)>0 THEN length(d.content)
                             ELSE COALESCE(SUM(length(c.content)),0) END
                 FROM documents d LEFT JOIN chunks c ON c.document_id=d.id
                 WHERE d.id=?1 GROUP BY d.id",
                params![
                    id,
                    i64::try_from(max_content_bytes.saturating_add(4)).unwrap_or(i64::MAX)
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, Option<Vec<u8>>>(8)?.unwrap_or_default(),
                        row.get::<_, i64>(9)?,
                        row.get::<_, i64>(10)?,
                        row.get::<_, i64>(11)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            source,
            source_id,
            title,
            uri,
            updated_at,
            project,
            acl_json,
            metadata_json,
            stored_content,
            stored_content_bytes,
            chunk_count,
            content_chars,
        )) = record
        else {
            return Ok(None);
        };
        let acl: Vec<String> = serde_json::from_str(&acl_json)?;
        if !acl_allows(&acl, principal_acl) {
            return Ok(None);
        }
        let metadata = serde_json::from_str(&metadata_json)?;
        let backlinks = document_backlinks(
            &connection,
            id,
            &source,
            &source_id,
            uri.as_deref(),
            &project,
            principal_acl,
        )?;
        let surrounding = surrounding_documents(&connection, id, &source, &project, principal_acl)?;
        let (content, truncated) = if stored_content.is_empty() {
            let legacy_fetch_limit = max_content_bytes.saturating_add(512);
            let mut statement = connection.prepare(
                "SELECT CAST(substr(CAST(content AS BLOB),1,?2) AS BLOB),
                        length(CAST(content AS BLOB))
                 FROM chunks WHERE document_id=?1 ORDER BY ordinal",
            )?;
            let mut rows = statement.query(params![
                id,
                i64::try_from(legacy_fetch_limit).unwrap_or(i64::MAX)
            ])?;
            reconstruct_chunk_rows(&mut rows, max_content_bytes)?
        } else {
            let truncated =
                usize::try_from(stored_content_bytes).unwrap_or(usize::MAX) > max_content_bytes;
            (
                bounded_utf8_bytes(stored_content, max_content_bytes),
                truncated,
            )
        };
        Ok(Some(DocumentDetail {
            summary: DocumentSummary {
                id: id.to_string(),
                source,
                source_id,
                title,
                uri,
                updated_at,
                project,
                chunk_count: usize::try_from(chunk_count).unwrap_or(usize::MAX),
                content_chars: usize::try_from(content_chars).unwrap_or(usize::MAX),
            },
            content,
            metadata,
            acl,
            backlinks,
            surrounding,
            truncated,
        }))
    }

    pub fn all_chunks(
        &self,
        project: Option<&str>,
        source: Option<&str>,
    ) -> Result<Vec<StoredChunk>> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(
            "SELECT c.id,d.source,d.source_id,d.title,d.uri,c.content,d.acl_json,
                    c.embedding_blob,c.embedding_json,d.updated_at
             FROM chunks c JOIN documents d ON d.id=c.document_id
             WHERE (?1 IS NULL OR d.project=?1) AND (?2 IS NULL OR d.source=?2)",
        )?;
        let rows = statement.query_map(params![project, source], row_to_chunk)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn semantic_ids(
        &self,
        query_embedding: &[f32],
        project: Option<&str>,
        source: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, f32)>> {
        self.semantic_ids_scoped(query_embedding, project, source, limit, &["*".into()])
    }

    pub fn semantic_ids_scoped(
        &self,
        query_embedding: &[f32],
        project: Option<&str>,
        source: Option<&str>,
        limit: usize,
        principal_acl: &[String],
    ) -> Result<Vec<(String, f32)>> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(
            "SELECT c.id,c.embedding_blob,c.embedding_json,d.acl_json
             FROM chunks c JOIN documents d ON d.id=c.document_id
             WHERE (?1 IS NULL OR d.project=?1) AND (?2 IS NULL OR d.source=?2)",
        )?;
        let rows = statement.query_map(params![project, source], |row| {
            let id = row.get::<_, String>(0)?;
            let blob = row.get::<_, Option<Vec<u8>>>(1)?;
            let json = row.get::<_, String>(2)?;
            let acl = serde_json::from_str::<Vec<String>>(&row.get::<_, String>(3)?).map_err(
                |error| rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error)),
            )?;
            let embedding = blob.map_or_else(
                || {
                    serde_json::from_str(&json).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(error))
                    })
                },
                |blob| {
                    decode_embedding(&blob).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(1, Type::Blob, error.into())
                    })
                },
            )?;
            Ok((id, cosine(query_embedding, &embedding), acl))
        })?;
        let mut best = BinaryHeap::<Reverse<SemanticCandidate>>::with_capacity(limit);
        for row in rows {
            let (id, score, acl) = row?;
            if !acl_allows(&acl, principal_acl) {
                continue;
            }
            let candidate = SemanticCandidate { id, score };
            if best.len() < limit {
                best.push(Reverse(candidate));
            } else if best.peek().is_some_and(|current| candidate > current.0) {
                best.pop();
                best.push(Reverse(candidate));
            }
        }
        let mut ranked = best
            .into_iter()
            .map(|Reverse(candidate)| (candidate.id, candidate.score))
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        Ok(ranked)
    }

    pub fn chunks_by_ids(&self, ids: &[String]) -> Result<Vec<StoredChunk>> {
        self.chunks_by_ids_scoped(ids, &["*".into()])
    }

    pub fn chunks_by_ids_scoped(
        &self,
        ids: &[String],
        principal_acl: &[String],
    ) -> Result<Vec<StoredChunk>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT c.id,d.source,d.source_id,d.title,d.uri,c.content,d.acl_json,
                    c.embedding_blob,c.embedding_json,d.updated_at
             FROM chunks c JOIN documents d ON d.id=c.document_id
             WHERE c.id IN ({placeholders})"
        );
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(ids), row_to_chunk)?;
        rows.filter_map(|row| match row {
            Ok(chunk) if acl_allows(&chunk.acl, principal_acl) => Some(Ok(chunk)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
    }

    pub fn neighboring_content_scoped(
        &self,
        ids: &[String],
        radius: usize,
        max_content_bytes: usize,
        principal_acl: &[String],
    ) -> Result<HashMap<String, String>> {
        if ids.is_empty() || max_content_bytes == 0 {
            return Ok(HashMap::new());
        }
        let radius = radius.min(4);
        let maximum = max_content_bytes.min(64 * 1024);
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT seed.id,n.content,d.acl_json
             FROM chunks seed
             JOIN chunks n ON n.document_id=seed.document_id
             JOIN documents d ON d.id=seed.document_id
             WHERE seed.id IN ({placeholders})
               AND n.ordinal BETWEEN seed.ordinal-{radius} AND seed.ordinal+{radius}
             ORDER BY seed.id,n.ordinal"
        );
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(ids), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut expanded = HashMap::<String, String>::new();
        let mut complete = HashSet::<String>::new();
        for row in rows {
            let (seed_id, content, acl_json) = row?;
            if complete.contains(&seed_id) {
                continue;
            }
            let acl = serde_json::from_str::<Vec<String>>(&acl_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(error))
            })?;
            if !acl_allows(&acl, principal_acl) {
                complete.insert(seed_id);
                continue;
            }
            let value = expanded.entry(seed_id.clone()).or_default();
            append_reconstructed_chunk(value, &content);
            if value.len() > maximum {
                value.truncate(previous_char_boundary(value, maximum));
                complete.insert(seed_id);
            }
        }
        Ok(expanded)
    }

    pub fn lexical_ids(
        &self,
        query: &str,
        project: Option<&str>,
        source: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>> {
        self.lexical_ids_scoped(query, project, source, limit, &["*".into()])
    }

    pub fn lexical_ids_scoped(
        &self,
        query: &str,
        project: Option<&str>,
        source: Option<&str>,
        limit: usize,
        principal_acl: &[String],
    ) -> Result<Vec<String>> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let terms = lexical_query_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let disjunction = terms.join(" OR ");
        let mut searches = Vec::new();
        if terms.len() > 1 {
            if terms.len() > 2 {
                searches.push(terms.join(" AND "));
            }
            for right in 1..terms.len().min(12) {
                searches.push(format!("{} AND {}", terms[0], terms[right]));
            }
        }
        searches.push(disjunction);
        let mut statement = connection.prepare(
            "SELECT f.chunk_id,d.acl_json FROM chunks_fts f
             JOIN chunks c ON c.id=f.chunk_id
             JOIN documents d ON d.id=c.document_id
             WHERE chunks_fts MATCH ?1
               AND (?2 IS NULL OR d.project=?2)
               AND (?3 IS NULL OR d.source=?3)
             ORDER BY bm25(chunks_fts) LIMIT ?4",
        )?;
        let candidate_limit = limit.saturating_mul(8).max(limit);
        let mut allowed = Vec::new();
        let mut seen = HashSet::new();
        for safe_query in searches {
            let rows = statement.query_map(
                params![safe_query, project, source, candidate_limit as i64],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?;
            for row in rows {
                let (id, acl) = row?;
                let acl: Vec<String> = serde_json::from_str(&acl)?;
                if acl_allows(&acl, principal_acl) && seen.insert(id.clone()) {
                    allowed.push(id);
                    if allowed.len() == limit {
                        return Ok(allowed);
                    }
                }
            }
        }
        Ok(allowed)
    }

    pub fn stats(&self) -> Result<StoreStats> {
        // Status and readiness are control-plane endpoints. Use a dedicated
        // read-only connection so a long-running ingestion/retrieval query
        // cannot hold the process-wide writer connection mutex and make
        // health checks wait behind it.
        let connection = self
            .read_connection
            .lock()
            .expect("read connection lock poisoned");
        let documents =
            connection.query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))?;
        let chunks = connection.query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))?;
        let embedding_fingerprint = connection
            .query_row(
                "SELECT value FROM meta WHERE key='embedding_fingerprint'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let (embedding_cache_entries, embedding_cache_hits) = connection.query_row(
            "SELECT COUNT(*),COALESCE(SUM(hits),0) FROM embedding_cache",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let (query_cache_entries, query_cache_hits) = connection.query_row(
            "SELECT COUNT(*),COALESCE(SUM(hits),0) FROM query_cache",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let mut statement = connection.prepare(
            "SELECT d.source,d.project,COUNT(DISTINCT d.id),COUNT(c.id),MAX(d.updated_at)
             FROM documents d LEFT JOIN chunks c ON c.document_id=d.id
             GROUP BY d.source,d.project ORDER BY d.project,d.source",
        )?;
        let sources = statement
            .query_map([], |row| {
                Ok(SourceStats {
                    source: row.get(0)?,
                    project: row.get(1)?,
                    documents: row.get(2)?,
                    chunks: row.get(3)?,
                    latest_updated_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut sync_statement = connection.prepare(
            "SELECT source,project,status,started_at,completed_at,documents,bytes,deleted,
                    budget_documents,budget_bytes,budget_seconds
             FROM (
               SELECT sync_runs.*,
                      ROW_NUMBER() OVER (
                        PARTITION BY source,project
                        ORDER BY started_at DESC,rowid DESC
                      ) AS rank
               FROM sync_runs
             )
             WHERE rank=1
             ORDER BY project,source",
        )?;
        let sync_runs = sync_statement
            .query_map([], |row| {
                Ok(SourceSyncStats {
                    source: row.get(0)?,
                    project: row.get(1)?,
                    status: row.get(2)?,
                    started_at: row.get(3)?,
                    completed_at: row.get(4)?,
                    documents: row.get(5)?,
                    bytes: row.get(6)?,
                    deleted: row.get(7)?,
                    budget_documents: row.get(8)?,
                    budget_bytes: row.get(9)?,
                    budget_seconds: row.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(StoreStats {
            documents,
            chunks,
            embedding_fingerprint,
            embedding_cache_entries,
            embedding_cache_hits,
            query_cache_entries,
            query_cache_hits,
            sources,
            sync_runs,
        })
    }

    /// Scoped variant of [`Self::stats`] counting only ACL-visible documents
    /// and their sources. `allowed_sync_sources` carries the canonical
    /// (source, project) keys of ACL-visible configured sources so sync
    /// outcomes stay visible for authorized sources that have not indexed any
    /// documents yet; runs for sources outside both sets are omitted.
    pub fn stats_scoped(
        &self,
        principal_acl: &[String],
        allowed_sync_sources: &HashSet<(String, String)>,
    ) -> Result<StoreStats> {
        let connection = self
            .read_connection
            .lock()
            .expect("read connection lock poisoned");
        let embedding_fingerprint = connection
            .query_row(
                "SELECT value FROM meta WHERE key='embedding_fingerprint'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let (embedding_cache_entries, embedding_cache_hits) = connection.query_row(
            "SELECT COUNT(*),COALESCE(SUM(hits),0) FROM embedding_cache",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let (query_cache_entries, query_cache_hits) = connection.query_row(
            "SELECT COUNT(*),COALESCE(SUM(hits),0) FROM query_cache",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        // Keep ACL filtering in SQLite so status does one grouped read rather
        // than materializing and parsing every document's ACL in Rust while
        // holding the store mutex. The JSON shape checks mirror
        // `serde_json::from_str::<Vec<String>>`: malformed/non-array/mixed-type
        // ACL values remain hidden instead of becoming public documents.
        let principal_acl_json = serde_json::to_string(principal_acl)?;
        let mut source_statement = connection.prepare(
            "WITH visible_documents AS (
               SELECT d.id,d.source,d.project,d.updated_at
               FROM documents d
               WHERE json_valid(d.acl_json)
                 AND json_type(d.acl_json)='array'
                 AND NOT EXISTS (
                   SELECT 1 FROM json_each(d.acl_json) AS document_acl
                   WHERE document_acl.type<>'text'
                 )
                 AND (
                   json_array_length(d.acl_json)=0
                   OR EXISTS (
                     SELECT 1 FROM json_each(?1) AS principal_acl
                     WHERE principal_acl.type='text'
                       AND (
                         principal_acl.value='*'
                         OR EXISTS (
                           SELECT 1 FROM json_each(d.acl_json) AS document_acl
                           WHERE document_acl.type='text'
                             AND document_acl.value=principal_acl.value
                         )
                       )
                   )
                 )
             )
             SELECT visible_documents.source,visible_documents.project,
                    COUNT(DISTINCT visible_documents.id),COUNT(c.id),
                    MAX(visible_documents.updated_at)
             FROM visible_documents
             LEFT JOIN chunks c ON c.document_id=visible_documents.id
             GROUP BY visible_documents.source,visible_documents.project
             ORDER BY visible_documents.source,visible_documents.project",
        )?;
        let sources = source_statement
            .query_map([principal_acl_json], |row| {
                Ok(SourceStats {
                    source: row.get(0)?,
                    project: row.get(1)?,
                    documents: row.get(2)?,
                    chunks: row.get(3)?,
                    latest_updated_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let documents = sources.iter().map(|source| source.documents).sum();
        let chunks = sources.iter().map(|source| source.chunks).sum();
        let allowed_sources = sources
            .iter()
            .map(|source| (source.source.clone(), source.project.clone()))
            .collect::<HashSet<_>>();
        let mut sync_statement = connection.prepare(
            "SELECT source,project,status,started_at,completed_at,documents,bytes,deleted,
                    budget_documents,budget_bytes,budget_seconds
             FROM (
               SELECT sync_runs.*,
                      ROW_NUMBER() OVER (
                        PARTITION BY source,project
                        ORDER BY started_at DESC,rowid DESC
                      ) AS rank
               FROM sync_runs
             )
             WHERE rank=1
             ORDER BY project,source",
        )?;
        let sync_runs = sync_statement
            .query_map([], |row| {
                Ok(SourceSyncStats {
                    source: row.get(0)?,
                    project: row.get(1)?,
                    status: row.get(2)?,
                    started_at: row.get(3)?,
                    completed_at: row.get(4)?,
                    documents: row.get(5)?,
                    bytes: row.get(6)?,
                    deleted: row.get(7)?,
                    budget_documents: row.get(8)?,
                    budget_bytes: row.get(9)?,
                    budget_seconds: row.get(10)?,
                })
            })?
            .filter_map(|row| match row {
                Ok(sync) => {
                    let key = (sync.source.clone(), sync.project.clone());
                    if allowed_sources.contains(&key) || allowed_sync_sources.contains(&key) {
                        Some(Ok(sync))
                    } else {
                        None
                    }
                }
                Err(error) => Some(Err(error)),
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(StoreStats {
            documents,
            chunks,
            embedding_fingerprint,
            embedding_cache_entries,
            embedding_cache_hits,
            query_cache_entries,
            query_cache_hits,
            sources,
            sync_runs,
        })
    }

    pub fn public_acl_summary(&self) -> Result<Vec<PublicAclSummary>> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(
            "SELECT project,COUNT(*) FROM documents
             WHERE acl_json='[]' GROUP BY project ORDER BY project",
        )?;
        statement
            .query_map([], |row| {
                Ok(PublicAclSummary {
                    project: row.get(0)?,
                    documents: usize::try_from(row.get::<_, i64>(1)?).unwrap_or(usize::MAX),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn backfill_project_acls(&self, mappings: &[(String, Vec<String>)]) -> Result<usize> {
        let mut connection = self.connection.lock().expect("store lock poisoned");
        let transaction = connection.transaction()?;
        let mut changed = 0_usize;
        for (project, labels) in mappings {
            anyhow::ensure!(!labels.is_empty(), "ACL labels must not be empty");
            changed = changed.saturating_add(transaction.execute(
                "UPDATE documents SET acl_json=?2 WHERE project=?1 AND acl_json='[]'",
                params![project, serde_json::to_string(labels)?],
            )?);
        }
        if changed > 0 {
            bump_corpus_revision(&transaction)?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    pub fn cached_embedding(&self, fingerprint: &str, content: &str) -> Result<Option<Vec<f32>>> {
        let hash = hex_digest(content.as_bytes());
        let connection = self.connection.lock().expect("store lock poisoned");
        let value: Option<(Option<Vec<u8>>, String)> = connection
            .query_row(
                "SELECT embedding_blob,embedding_json FROM embedding_cache
                 WHERE fingerprint=?1 AND content_hash=?2",
                params![fingerprint, hash],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if value.is_some() {
            optional_write(&connection, || {
                connection.execute(
                    "UPDATE embedding_cache SET hits=hits+1,last_used_at=?3
                     WHERE fingerprint=?1 AND content_hash=?2",
                    params![fingerprint, hash, Utc::now().to_rfc3339()],
                )
            })?;
        }
        value
            .map(|(blob, json)| {
                blob.map_or_else(
                    || serde_json::from_str(&json).map_err(Into::into),
                    |blob| decode_embedding(&blob),
                )
            })
            .transpose()
    }

    pub fn cache_embedding(
        &self,
        fingerprint: &str,
        content: &str,
        embedding: &[f32],
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let connection = self.connection.lock().expect("store lock poisoned");
        connection.execute(
            "INSERT INTO embedding_cache(
               fingerprint,content_hash,embedding_json,embedding_blob,hits,created_at,last_used_at)
             VALUES(?1,?2,'[]',?3,0,?4,?4)
             ON CONFLICT(fingerprint,content_hash) DO UPDATE SET
               embedding_json='[]',embedding_blob=excluded.embedding_blob,
               last_used_at=excluded.last_used_at",
            params![
                fingerprint,
                hex_digest(content.as_bytes()),
                encode_embedding(embedding),
                now
            ],
        )?;
        Ok(())
    }

    pub fn cache_embedding_if_available(
        &self,
        fingerprint: &str,
        content: &str,
        embedding: &[f32],
    ) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let connection = self.connection.lock().expect("store lock poisoned");
        optional_write(&connection, || {
            connection.execute(
                "INSERT INTO embedding_cache(
                   fingerprint,content_hash,embedding_json,embedding_blob,hits,created_at,last_used_at)
                 VALUES(?1,?2,'[]',?3,0,?4,?4)
                 ON CONFLICT(fingerprint,content_hash) DO UPDATE SET
                   embedding_json='[]',embedding_blob=excluded.embedding_blob,
                   last_used_at=excluded.last_used_at",
                params![
                    fingerprint,
                    hex_digest(content.as_bytes()),
                    encode_embedding(embedding),
                    now
                ],
            )
        })
        .map(|result| result.is_some())
    }

    pub fn prune_embedding_cache(&self, max_entries: usize) -> Result<usize> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let count: i64 =
            connection.query_row("SELECT COUNT(*) FROM embedding_cache", [], |row| row.get(0))?;
        let maximum = i64::try_from(max_entries).unwrap_or(i64::MAX);
        let remove = (count - maximum).max(0);
        if remove == 0 {
            return Ok(0);
        }
        let deleted = connection.execute(
            "DELETE FROM embedding_cache WHERE rowid IN (
               SELECT rowid FROM embedding_cache
               ORDER BY last_used_at ASC,created_at ASC LIMIT ?1
             )",
            [remove],
        )?;
        Ok(deleted)
    }

    pub fn integrity_check(&self) -> Result<()> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let result: String =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        anyhow::ensure!(result == "ok", "database integrity check failed: {result}");
        Ok(())
    }

    pub fn backup(&self, destination: &Path) -> Result<()> {
        reject_database_symlinks(destination)?;
        anyhow::ensure!(
            !destination.exists(),
            "backup already exists: {}",
            destination.display()
        );
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = destination.with_extension(format!("{}.partial", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let connection = self.connection.lock().expect("store lock poisoned");
            connection.execute("VACUUM INTO ?1", [temporary.to_string_lossy().as_ref()])?;
            drop(connection);
            secure_file(&temporary)?;
            verify_database(&temporary)?;
            std::fs::rename(&temporary, destination)?;
            Ok(())
        })();
        if result.is_err() && temporary.exists() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    pub fn verify(path: &Path) -> Result<()> {
        verify_database(path)
    }

    pub fn restore(database: &Path, source: &Path, recovery_backup: Option<&Path>) -> Result<()> {
        reject_database_symlinks(database)?;
        reject_database_symlinks(source)?;
        if let Some(recovery) = recovery_backup {
            reject_database_symlinks(recovery)?;
        }
        verify_database(source)?;
        if database.exists()
            && let Some(recovery) = recovery_backup
        {
            Self::open(database)?.backup(recovery)?;
        }
        if let Some(parent) = database.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let source =
            Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut destination = Connection::open(database)?;
        let backup = rusqlite::backup::Backup::new(&source, &mut destination)?;
        backup.run_to_completion(128, Duration::from_millis(10), None)?;
        drop(backup);
        drop(destination);
        secure_database_files(database)?;
        verify_database(database)
    }
}

fn backfill_document_links(connection: &mut Connection) -> Result<()> {
    let indexed: Option<String> = connection
        .query_row(
            "SELECT value FROM meta WHERE key='document_links_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if indexed.as_deref() == Some("1") {
        return Ok(());
    }
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT OR IGNORE INTO document_links(document_id,target)
         SELECT document_id,target FROM (
           SELECT d.id AS document_id,j.value AS target,
                  row_number() OVER (PARTITION BY d.id ORDER BY j.fullkey) AS ordinal
           FROM documents d,json_tree(d.metadata_json) j
           WHERE j.type='text' AND length(j.value) BETWEEN 1 AND 4096
             AND (
               lower(CAST(j.key AS TEXT)) IN (
                 'uri','url','link','links','ref','refs','reference','references',
                 'related','source_id','document_id','parent_uri','parent_id'
               )
               OR lower(j.path) IN (
                 '$.links','$.refs','$.references','$.related'
               )
               OR lower(j.path) LIKE '%.links'
               OR lower(j.path) LIKE '%.refs'
               OR lower(j.path) LIKE '%.references'
               OR lower(j.path) LIKE '%.related'
             )
         ) WHERE ordinal<=256",
        [],
    )?;
    transaction.execute(
        "INSERT INTO meta(key,value) VALUES('document_links_version','1')
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [],
    )?;
    transaction.commit()?;
    Ok(())
}

fn document_backlinks(
    connection: &Connection,
    id: &str,
    source: &str,
    source_id: &str,
    uri: Option<&str>,
    project: &str,
    principal_acl: &[String],
) -> Result<Vec<DocumentReference>> {
    let mut statement = connection.prepare(
        "SELECT d.id,d.source,d.source_id,d.title,d.uri,d.updated_at,d.project,d.acl_json
         FROM document_links l JOIN documents d ON d.id=l.document_id
         WHERE d.id<>?1 AND d.project=?2
           AND ((d.source=?3 AND l.target=?4) OR (?5 IS NOT NULL AND l.target=?5))
         GROUP BY d.id
         ORDER BY d.updated_at DESC,d.id DESC
         LIMIT 200",
    )?;
    let rows = statement
        .query_map(params![id, project, source, source_id, uri], |row| {
            Ok((
                DocumentReference {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    source_id: row.get(2)?,
                    title: row.get(3)?,
                    uri: row.get(4)?,
                    updated_at: row.get(5)?,
                    project: row.get(6)?,
                },
                row.get::<_, String>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut references = Vec::with_capacity(12);
    for (reference, acl_json) in rows {
        let acl: Vec<String> = serde_json::from_str(&acl_json)?;
        if acl_allows(&acl, principal_acl) {
            references.push(reference);
            if references.len() == 12 {
                break;
            }
        }
    }
    Ok(references)
}

fn metadata_reference_strings(value: &Value) -> Vec<String> {
    let mut values = HashSet::new();
    collect_metadata_reference_strings(value, &mut values);
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort();
    values
}

fn collect_metadata_reference_strings(value: &Value, values: &mut HashSet<String>) {
    if values.len() >= 256 {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                collect_metadata_reference_strings(item, values);
            }
        }
        Value::Object(items) => {
            for (key, item) in items {
                if is_reference_metadata_key(key) {
                    collect_reference_values(item, values);
                } else {
                    collect_metadata_reference_strings(item, values);
                }
            }
        }
        _ => {}
    }
}

fn is_reference_metadata_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "uri"
            | "url"
            | "link"
            | "links"
            | "ref"
            | "refs"
            | "reference"
            | "references"
            | "related"
            | "source_id"
            | "document_id"
            | "parent_uri"
            | "parent_id"
    )
}

fn collect_reference_values(value: &Value, values: &mut HashSet<String>) {
    if values.len() >= 256 {
        return;
    }
    match value {
        Value::String(value) if !value.is_empty() && value.len() <= 4096 => {
            values.insert(value.clone());
        }
        Value::Array(items) => {
            for item in items {
                collect_reference_values(item, values);
            }
        }
        Value::Object(items) => {
            for item in items.values() {
                collect_reference_values(item, values);
            }
        }
        _ => {}
    }
}

fn surrounding_documents(
    connection: &Connection,
    id: &str,
    source: &str,
    project: &str,
    principal_acl: &[String],
) -> Result<Vec<DocumentReference>> {
    let mut statement = connection.prepare(
        "SELECT d.id,d.source,d.source_id,d.title,d.uri,d.updated_at,d.project,d.acl_json
         FROM documents d
         WHERE d.id<>?1 AND d.source=?2 AND d.project=?3
         ORDER BY abs(julianday(d.updated_at)-julianday(
             (SELECT updated_at FROM documents WHERE id=?1)
         )),d.updated_at DESC,d.id DESC
         LIMIT 64",
    )?;
    let rows = statement
        .query_map(params![id, source, project], |row| {
            Ok((
                DocumentReference {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    source_id: row.get(2)?,
                    title: row.get(3)?,
                    uri: row.get(4)?,
                    updated_at: row.get(5)?,
                    project: row.get(6)?,
                },
                row.get::<_, String>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut references = Vec::with_capacity(8);
    for (reference, acl_json) in rows {
        let acl: Vec<String> = serde_json::from_str(&acl_json)?;
        if acl_allows(&acl, principal_acl) {
            references.push(reference);
            if references.len() == 8 {
                break;
            }
        }
    }
    Ok(references)
}

fn optional_write<T>(
    connection: &Connection,
    operation: impl FnOnce() -> rusqlite::Result<T>,
) -> Result<Option<T>> {
    connection.busy_timeout(Duration::ZERO)?;
    let result = operation();
    connection.busy_timeout(DATABASE_BUSY_TIMEOUT)?;
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error)
            if matches!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn verify_database(path: &Path) -> Result<()> {
    reject_database_symlinks(path)?;
    anyhow::ensure!(
        path.is_file(),
        "database does not exist: {}",
        path.display()
    );
    let metadata = std::fs::metadata(path)?;
    anyhow::ensure!(metadata.len() > 0, "database is empty: {}", path.display());
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let result: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    anyhow::ensure!(result == "ok", "database integrity check failed: {result}");
    Ok(())
}

#[cfg(unix)]
fn secure_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    reject_symlink(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to secure {}", path.display()))
}

#[cfg(not(unix))]
fn secure_file(_path: &Path) -> Result<()> {
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("refusing to use symlinked database path {}", path.display());
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect database path {}", path.display())),
    }
}

fn reject_database_symlinks(database: &Path) -> Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let mut path = database.as_os_str().to_os_string();
        path.push(suffix);
        reject_symlink(&PathBuf::from(path))?;
    }
    Ok(())
}

fn secure_database_files(database: &Path) -> Result<()> {
    reject_database_symlinks(database)?;
    for suffix in ["", "-wal", "-shm"] {
        let mut path = database.as_os_str().to_os_string();
        path.push(suffix);
        let path = PathBuf::from(path);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => secure_file(&path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect database sidecar {}", path.display())
                });
            }
        }
    }
    Ok(())
}

struct SemanticCandidate {
    id: String,
    score: f32,
}

impl PartialEq for SemanticCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.score.to_bits() == other.score.to_bits() && self.id == other.id
    }
}

impl Eq for SemanticCandidate {}

impl PartialOrd for SemanticCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SemanticCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| self.id.cmp(&other.id))
    }
}

fn row_to_chunk(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredChunk> {
    let acl_json: String = row.get(6)?;
    let acl = serde_json::from_str(&acl_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(error))
    })?;
    let embedding_blob: Option<Vec<u8>> = row.get(7)?;
    let embedding_json: String = row.get(8)?;
    let embedding = embedding_blob.map_or_else(
        || {
            serde_json::from_str(&embedding_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(error))
            })
        },
        |blob| {
            decode_embedding(&blob).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(7, Type::Blob, error.into())
            })
        },
    )?;
    let updated_at: String = row.get(9)?;
    Ok(StoredChunk {
        id: row.get(0)?,
        source: row.get(1)?,
        source_id: row.get(2)?,
        title: row.get(3)?,
        uri: row.get(4)?,
        content: row.get(5)?,
        acl,
        embedding,
        updated_at: DateTime::parse_from_rfc3339(&updated_at)
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
}

fn migrate_embedding_blobs(connection: &mut Connection) -> Result<()> {
    ensure_column(connection, "chunks", "embedding_blob", "BLOB")?;
    ensure_column(connection, "embedding_cache", "embedding_blob", "BLOB")?;
    for table in ["chunks", "embedding_cache"] {
        let rows = {
            let mut statement = connection.prepare(&format!(
                "SELECT rowid,embedding_json FROM {table}
                 WHERE embedding_blob IS NULL"
            ))?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        if rows.is_empty() {
            continue;
        }
        let transaction = connection.transaction()?;
        for (rowid, json) in rows {
            let embedding: Vec<f32> = serde_json::from_str(&json)
                .with_context(|| format!("invalid legacy embedding in {table} row {rowid}"))?;
            transaction.execute(
                &format!("UPDATE {table} SET embedding_blob=?2,embedding_json='[]' WHERE rowid=?1"),
                params![rowid, encode_embedding(&embedding)],
            )?;
        }
        transaction.commit()?;
    }
    Ok(())
}

fn ensure_document_content_column(connection: &Connection) -> Result<()> {
    ensure_column(
        connection,
        "documents",
        "content",
        "TEXT NOT NULL DEFAULT ''",
    )
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    declaration: &str,
) -> Result<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    if !columns.iter().any(|existing| existing == column) {
        connection.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {declaration}"
        ))?;
    }
    Ok(())
}

fn reconstruct_chunk_rows(rows: &mut rusqlite::Rows<'_>, maximum: usize) -> Result<(String, bool)> {
    let mut content = String::new();
    while let Some(row) = rows.next()? {
        let chunk_bytes = row.get::<_, Vec<u8>>(0)?;
        let original_bytes = usize::try_from(row.get::<_, i64>(1)?).unwrap_or(usize::MAX);
        let chunk = bounded_utf8_bytes(chunk_bytes, maximum.saturating_add(512));
        append_reconstructed_chunk(&mut content, &chunk);
        if content.len() > maximum || original_bytes > maximum.saturating_add(512) {
            content.truncate(previous_char_boundary(&content, maximum));
            return Ok((content, true));
        }
    }
    Ok((content, false))
}

fn append_reconstructed_chunk(content: &mut String, chunk: &str) {
    if content.is_empty() {
        content.push_str(chunk);
        return;
    }
    let maximum = content.len().min(chunk.len()).min(512);
    let overlap = (1..=maximum)
        .rev()
        .find(|size| {
            content.is_char_boundary(content.len() - size)
                && chunk.is_char_boundary(*size)
                && content[content.len() - size..] == chunk[..*size]
        })
        .unwrap_or_default();
    if overlap == 0 {
        content.push_str("\n\n");
    }
    content.push_str(&chunk[overlap..]);
}

fn bounded_utf8_bytes(mut content: Vec<u8>, maximum: usize) -> String {
    content.truncate(maximum);
    while std::str::from_utf8(&content).is_err() {
        content.pop();
    }
    String::from_utf8(content).expect("validated UTF-8")
}

fn previous_char_boundary(content: &str, maximum: usize) -> usize {
    let mut end = maximum.min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn encode_embedding(embedding: &[f32]) -> Vec<u8> {
    embedding
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn decode_embedding(value: &[u8]) -> Result<Vec<f32>> {
    anyhow::ensure!(
        value.len().is_multiple_of(std::mem::size_of::<f32>()),
        "embedding blob length is not divisible by four"
    );
    Ok(value
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm * right_norm)
    }
}

fn memory_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemorySearchResult> {
    Ok(MemorySearchResult {
        memory: memory_record_from_row(row)?,
        lexical_score: row.get(19)?,
    })
}

fn memory_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRecord> {
    let acl_json: String = row.get(11)?;
    let provenance_json: String = row.get(12)?;
    Ok(MemoryRecord {
        id: row.get(0)?,
        kind: row.get(1)?,
        project: row.get(2)?,
        title: row.get(3)?,
        content: row.get(4)?,
        source: row.get(5)?,
        source_id: row.get(6)?,
        dedupe_key: row.get(7)?,
        confidence: row.get(8)?,
        importance: row.get(9)?,
        status: row.get(10)?,
        acl: serde_json::from_str(&acl_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(11, Type::Text, Box::new(error))
        })?,
        provenance: serde_json::from_str(&provenance_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(12, Type::Text, Box::new(error))
        })?,
        observed_at: row.get(13)?,
        valid_from: row.get(14)?,
        valid_until: row.get(15)?,
        supersedes_id: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
    })
}

fn stable_id(source: &str, source_id: &str) -> String {
    hex_digest(format!("{source}\0{source_id}").as_bytes())
}

fn hex_digest(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn document_hash(document: &Document) -> Result<String> {
    let value = serde_json::to_vec(&serde_json::json!({
        "title": document.title,
        "content": document.content,
        "uri": document.uri,
        "project": document.project,
        "acl": document.acl,
        "metadata": document.metadata,
    }))?;
    Ok(hex_digest(&value))
}

fn lexical_query_terms(query: &str) -> Vec<String> {
    const STOPWORDS: [&str; 30] = [
        "a", "an", "and", "are", "be", "can", "did", "do", "does", "for", "from", "how", "i", "in",
        "is", "it", "my", "of", "on", "or", "our", "should", "the", "this", "to", "was", "were",
        "what", "when", "with",
    ];
    let raw = query
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|character: char| {
                    !character.is_alphanumeric()
                        && character != '_'
                        && character != '-'
                        && character != '.'
                })
                .replace('"', "")
                .to_lowercase()
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let meaningful = raw
        .iter()
        .filter(|token| !STOPWORDS.contains(&token.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let selected = if meaningful.is_empty() {
        raw
    } else {
        meaningful
    };
    selected
        .into_iter()
        .map(|token| format!("\"{token}\""))
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::tempdir;

    use super::*;

    fn document(source_id: &str, content: &str) -> Document {
        Document {
            source: "test".into(),
            source_id: source_id.into(),
            title: source_id.into(),
            content: content.into(),
            uri: None,
            updated_at: Utc::now(),
            project: "demo".into(),
            acl: Vec::new(),
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn document_browser_is_acl_scoped_paginated_and_bounded() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let mut personal = document("personal", "personal exact content");
        personal.acl = vec!["personal".into()];
        personal.updated_at = "2026-01-03T00:00:00Z".parse().expect("timestamp");
        let mut work = document("work", "work exact content");
        work.acl = vec!["work".into()];
        work.updated_at = "2026-01-02T00:00:00Z".parse().expect("timestamp");
        let mut public = document("public", "public exact content");
        public.updated_at = "2026-01-01T00:00:00Z".parse().expect("timestamp");
        public.metadata = serde_json::json!({
            "references": ["work"],
            "access_token": "do-not-index-as-a-link"
        });
        for item in [&personal, &work, &public] {
            store
                .upsert(item, &[(item.content.clone(), vec![1.0])])
                .expect("insert document");
        }
        let secret_link_count: i64 = store
            .connection
            .lock()
            .expect("store lock")
            .query_row(
                "SELECT COUNT(*) FROM document_links WHERE target='do-not-index-as-a-link'",
                [],
                |row| row.get(0),
            )
            .expect("secret link count");
        assert_eq!(secret_link_count, 0);

        let first = store
            .list_documents_scoped(None, None, None, None, 1, &["work".into()])
            .expect("first page");
        assert_eq!(first.documents[0].title, "work");
        assert!(first.has_more);
        let cursor = DocumentCursor {
            updated_at: first.documents[0].updated_at.clone(),
            id: first.documents[0].id.clone(),
        };
        let second = store
            .list_documents_scoped(None, None, None, Some(&cursor), 1, &["work".into()])
            .expect("second page");
        assert_eq!(second.documents[0].title, "public");
        assert!(!second.has_more);

        assert!(
            store
                .document_scoped(&stable_id("test", "personal"), &["work".into()], 1024)
                .expect("denied detail")
                .is_none()
        );
        let detail = store
            .document_scoped(&stable_id("test", "work"), &["work".into()], 8)
            .expect("detail")
            .expect("visible detail");
        assert_eq!(detail.content, "work exa");
        assert!(detail.truncated);
        assert_eq!(detail.summary.source_id, "work");
        assert_eq!(detail.acl, ["work"]);
        assert_eq!(detail.backlinks[0].source_id, "public");
        assert_eq!(detail.surrounding[0].source_id, "public");
        assert_eq!(detail.summary.content_chars, work.content.chars().count());

        let filtered = store
            .list_documents_scoped(None, None, Some("WORK"), None, 10, &["work".into()])
            .expect("filtered page");
        assert_eq!(filtered.documents.len(), 1);
        assert_eq!(filtered.documents[0].source_id, "work");
    }

    #[test]
    fn legacy_document_browser_reconstructs_overlapping_chunks() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("store.sqlite3");
        let store = Store::open(&path).expect("open store");
        let item = document("legacy-browser", "alpha beta gamma");
        store
            .upsert(
                &item,
                &[
                    ("alpha beta".into(), vec![1.0]),
                    ("beta gamma".into(), vec![1.0]),
                ],
            )
            .expect("insert document");
        let connection = Connection::open(&path).expect("raw connection");
        connection
            .execute(
                "UPDATE documents SET content='' WHERE id=?1",
                [stable_id("test", "legacy-browser")],
            )
            .expect("simulate legacy row");
        drop(connection);
        assert!(
            store
                .needs_update(&item)
                .expect("legacy row requires content backfill")
        );

        let detail = store
            .document_scoped(&stable_id("test", "legacy-browser"), &["*".into()], 1024)
            .expect("detail")
            .expect("document");
        assert_eq!(detail.content, "alpha beta gamma");
        assert!(!detail.truncated);
    }

    #[test]
    fn neighboring_content_is_acl_scoped_overlap_aware_and_bounded() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let mut item = document("neighbor-context", "alpha beta gamma delta");
        item.acl = vec!["work".into()];
        store
            .upsert(
                &item,
                &[
                    ("alpha beta".into(), vec![1.0]),
                    ("beta gamma".into(), vec![1.0]),
                    ("gamma delta".into(), vec![1.0]),
                ],
            )
            .expect("insert document");
        let seed = format!("{}:1", stable_id("test", "neighbor-context"));

        let denied = store
            .neighboring_content_scoped(std::slice::from_ref(&seed), 1, 1024, &["personal".into()])
            .expect("denied context");
        assert!(denied.is_empty());

        let expanded = store
            .neighboring_content_scoped(std::slice::from_ref(&seed), 1, 1024, &["work".into()])
            .expect("expanded context");
        assert_eq!(expanded[&seed], "alpha beta gamma delta");

        let bounded = store
            .neighboring_content_scoped(&[seed.clone()], 1, 12, &["work".into()])
            .expect("bounded context");
        assert_eq!(bounded[&seed], "alpha beta g");
    }

    #[test]
    fn document_display_bound_preserves_unicode_boundaries() {
        assert_eq!(bounded_utf8_bytes("🧠memory".as_bytes().to_vec(), 5), "🧠m");
    }

    #[test]
    fn legacy_store_adds_canonical_content_column() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("legacy.sqlite3");
        let connection = Connection::open(&path).expect("legacy database");
        connection
            .execute_batch(
                "CREATE TABLE documents(
                   id TEXT PRIMARY KEY,source TEXT NOT NULL,source_id TEXT NOT NULL,
                   title TEXT NOT NULL,uri TEXT,content_hash TEXT NOT NULL,
                   updated_at TEXT NOT NULL,project TEXT NOT NULL,acl_json TEXT NOT NULL,
                   metadata_json TEXT NOT NULL,UNIQUE(source,source_id));",
            )
            .expect("legacy schema");
        connection
            .execute(
                "INSERT INTO documents(
                   id,source,source_id,title,uri,content_hash,updated_at,project,acl_json,metadata_json
                 ) VALUES('legacy-id','notes','legacy-source','Legacy',NULL,'hash',
                          '2026-01-01T00:00:00Z','demo','[]','{\"ref\":\"legacy-target\"}')",
                [],
            )
            .expect("legacy document");
        drop(connection);

        drop(Store::open(&path).expect("migrate store"));
        let connection = Connection::open(&path).expect("inspect store");
        let columns = connection
            .prepare("PRAGMA table_info(documents)")
            .expect("table info")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("columns")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("column names");
        assert!(columns.iter().any(|column| column == "content"));
        let link_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM document_links
                 WHERE document_id='legacy-id' AND target='legacy-target'",
                [],
                |row| row.get(0),
            )
            .expect("backfilled link");
        assert_eq!(link_count, 1);
    }

    #[test]
    fn unchanged_documents_skip_work_and_reconciliation_deletes_stale_rows() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let first = document("one", "first");
        let second = document("two", "second");
        assert_eq!(store.corpus_revision().expect("initial revision"), 0);
        assert!(store.needs_update(&first).expect("check first"));
        store
            .upsert(&first, &[("first".into(), vec![1.0])])
            .expect("insert first");
        assert_eq!(store.corpus_revision().expect("first revision"), 1);
        store
            .upsert(&second, &[("second".into(), vec![1.0])])
            .expect("insert second");
        assert_eq!(store.corpus_revision().expect("second revision"), 2);
        assert!(
            !store
                .upsert(&first, &[("first".into(), vec![1.0])])
                .expect("skip unchanged")
        );
        assert_eq!(store.corpus_revision().expect("unchanged revision"), 2);
        assert!(!store.needs_update(&first).expect("check unchanged"));
        let mut refreshed = first.clone();
        refreshed.updated_at += chrono::Duration::days(1);
        store
            .refresh_timestamp(&refreshed)
            .expect("refresh timestamp");
        assert_eq!(store.corpus_revision().expect("timestamp revision"), 3);
        assert_eq!(
            store.all_chunks(None, None).expect("chunks")[0].updated_at,
            refreshed.updated_at
        );

        let deleted = store
            .reconcile("test", "demo", &["one".into()])
            .expect("reconcile");
        assert_eq!(deleted, 1);
        assert_eq!(store.corpus_revision().expect("reconcile revision"), 4);
        assert_eq!(
            store
                .all_chunks(Some("demo"), Some("test"))
                .expect("remaining")
                .len(),
            1
        );
        let stats = store.stats().expect("stats");
        assert_eq!(stats.documents, 1);
        assert_eq!(stats.chunks, 1);
        assert_eq!(stats.sources[0].source, "test");
        assert_eq!(stats.embedding_cache_entries, 0);
    }

    #[test]
    fn scoped_stats_count_only_acl_visible_documents() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let mut work = document("work", "work content");
        work.acl = vec!["work".into()];
        let mut personal = document("personal", "personal content");
        personal.acl = vec!["personal".into()];
        let public = document("public", "public content");
        for item in [&work, &personal, &public] {
            store
                .upsert(item, &[(item.content.clone(), vec![1.0])])
                .expect("insert document");
        }
        let mut chunked = document("chunked", "chunked content");
        chunked.acl = vec!["work".into()];
        store
            .upsert(
                &chunked,
                &[
                    ("first chunk".into(), vec![1.0]),
                    ("second chunk".into(), vec![1.0]),
                ],
            )
            .expect("insert multi-chunk document");
        {
            let connection = store.connection.lock().expect("store lock");
            for (source_id, acl_json) in [
                ("malformed", "not-json"),
                ("mixed", "[\"work\",1]"),
                ("object", "{\"label\":\"work\"}"),
            ] {
                connection
                    .execute(
                        "INSERT INTO documents(
                           id,source,source_id,title,uri,content_hash,updated_at,project,
                           acl_json,metadata_json,content
                         ) VALUES(?1,'test',?2,?2,NULL,'hash','2026-01-04T00:00:00Z',
                                  'demo',?3,'{}','')",
                        rusqlite::params![stable_id("test", source_id), source_id, acl_json],
                    )
                    .expect("insert malformed ACL fixture");
            }
        }

        let stats = store
            .stats_scoped(&["work".into()], &HashSet::new())
            .expect("scoped stats");
        assert_eq!(stats.documents, 3);
        assert_eq!(stats.chunks, 4);
        assert_eq!(stats.sources.len(), 1);
        assert_eq!(stats.sources[0].documents, 3);
        assert_eq!(stats.sources[0].chunks, 4);
        assert_eq!(stats.sources[0].project, "demo");
    }

    #[test]
    fn scoped_stats_surface_runs_for_allowed_configured_sources_without_documents() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let failed = store
            .begin_sync("work-drive", "work", 100, 2_048, 30)
            .expect("begin failed sync");
        store
            .finish_sync(&failed, SyncRunStatus::Failed, None, None, None)
            .expect("finish failed sync");
        let running = store
            .begin_sync("personal-notes", "personal", 50, 1_024, 60)
            .expect("begin running sync");

        let allowed = HashSet::from([("work-drive".to_string(), "work".to_string())]);
        let stats = store
            .stats_scoped(&["work".into()], &allowed)
            .expect("scoped stats");
        assert_eq!(stats.documents, 0, "evidence counts stay document-derived");
        assert!(stats.sources.is_empty());
        assert_eq!(stats.sync_runs.len(), 1);
        let run = &stats.sync_runs[0];
        assert_eq!(run.source, "work-drive");
        assert_eq!(run.project, "work");
        assert_eq!(run.status, "failed");
        assert!(run.completed_at.is_some());
        assert_eq!(run.budget_documents, 100);

        store
            .finish_sync(&running, SyncRunStatus::Cancelled, None, None, None)
            .expect("finish running sync");
    }

    #[test]
    fn query_cache_tracks_hits_ttl_bounds_and_opt_out() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store.cache_query("one", "first", 1).expect("cache first");
        assert_eq!(
            store.cached_query("one", 60).expect("read first"),
            Some("first".into())
        );
        store.cache_query("two", "second", 1).expect("cache second");
        assert_eq!(store.cached_query("one", 60).expect("pruned first"), None);
        assert_eq!(
            store.cached_query("two", 60).expect("read second"),
            Some("second".into())
        );
        assert_eq!(store.cached_query("two", 0).expect("zero ttl"), None);
        store
            .cache_query("disabled", "ignored", 0)
            .expect("disabled cache");
        assert_eq!(
            store
                .cached_query("disabled", 60)
                .expect("disabled cache read"),
            None
        );
        let stats = store.stats().expect("cache stats");
        assert_eq!(stats.query_cache_entries, 1);
        assert_eq!(stats.query_cache_hits, 1);
    }

    #[test]
    fn malformed_query_cache_timestamps_are_evicted_as_misses() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let connection = store.connection.lock().expect("store lock");
        connection
            .execute(
                "INSERT INTO query_cache(cache_key,response_json,created_at,last_used_at,hits)
                 VALUES(?1,?2,?3,?3,0)",
                params!["malformed-time", "{}", "not-a-timestamp"],
            )
            .expect("malformed cache row");
        drop(connection);

        assert_eq!(
            store
                .cached_query("malformed-time", 3600)
                .expect("cache miss"),
            None
        );
        let connection = store.connection.lock().expect("store lock");
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM query_cache WHERE cache_key=?1",
                ["malformed-time"],
                |row| row.get(0),
            )
            .expect("cache count");
        assert_eq!(count, 0);
    }

    #[test]
    fn lexical_search_prioritizes_meaningful_multi_term_matches_over_stopwords() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .upsert(
                &document(
                    "relevant",
                    "Cortana ingestion uses bounded validation before a sync is enabled.",
                ),
                &[(
                    "Cortana ingestion uses bounded validation before a sync is enabled.".into(),
                    vec![1.0],
                )],
            )
            .expect("relevant document");
        store
            .upsert(
                &document(
                    "distractor",
                    "The analysis should be thoughtful and run only safe probes.",
                ),
                &[(
                    "The analysis should be thoughtful and run only safe probes.".into(),
                    vec![1.0],
                )],
            )
            .expect("distractor document");

        let ids = store
            .lexical_ids(
                "How should Cortana ingestion be run safely?",
                Some("demo"),
                None,
                10,
            )
            .expect("lexical search");
        assert_eq!(ids[0], format!("{}:0", stable_id("test", "relevant")));
    }

    #[test]
    fn audit_is_bounded_and_records_metadata_only() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        for index in 0..3 {
            assert!(
                store
                    .record_audit(
                        "agent",
                        "search",
                        Some("demo"),
                        Some("notes"),
                        "ok",
                        Some(index),
                        index as u64,
                        2,
                    )
                    .expect("record audit")
            );
        }

        let events = store.audit_events(500).expect("audit events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].result_count, Some(2));
        assert_eq!(events[1].result_count, Some(1));
        assert_eq!(events[0].principal, "agent");
        assert_eq!(events[0].project.as_deref(), Some("demo"));

        // Interactive API reads are newest-first and capped at 500, while an
        // operator export preserves the retained window in chronological order.
        let export = store
            .audit_events_for_export(2)
            .expect("audit export events");
        assert_eq!(export.len(), 2);
        assert_eq!(export[0].result_count, Some(1));
        assert_eq!(export[1].result_count, Some(2));

        assert!(
            !store
                .record_audit("agent", "answer", None, None, "ok", None, 0, 0)
                .expect("disabled audit")
        );
        assert_eq!(store.audit_events(500).expect("unchanged audit").len(), 2);
    }

    #[test]
    fn acl_backfill_is_explicit_bounded_and_invalidates_query_revision_once() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let work = document("work", "work document");
        let mut personal = document("personal", "personal document");
        personal.project = "personal".into();
        store
            .upsert(&work, &[("work document".into(), vec![1.0])])
            .expect("work document");
        store
            .upsert(&personal, &[("personal document".into(), vec![1.0])])
            .expect("personal document");
        let revision = store.corpus_revision().expect("revision");
        assert_eq!(store.public_acl_summary().expect("summary").len(), 2);

        assert_eq!(
            store
                .backfill_project_acls(&[("demo".into(), vec!["work".into()])])
                .expect("backfill"),
            1
        );
        assert_eq!(
            store.corpus_revision().expect("backfill revision"),
            revision + 1
        );
        let scoped = store
            .lexical_ids_scoped(
                "work document",
                Some("demo"),
                None,
                10,
                &["personal".into()],
            )
            .expect("scoped chunks");
        assert!(scoped.is_empty());
        assert_eq!(
            store
                .backfill_project_acls(&[("demo".into(), vec!["work".into()])])
                .expect("idempotent backfill"),
            0
        );
        assert_eq!(
            store.corpus_revision().expect("stable revision"),
            revision + 1
        );
    }

    #[test]
    fn sync_runs_persist_latest_source_outcome_and_budgets() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let interrupted = store
            .begin_sync("work-code", "work", 100, 2_048, 30)
            .expect("begin interrupted sync");
        let running = store.stats().expect("running stats");
        assert_eq!(running.sync_runs.len(), 1);
        assert_eq!(running.sync_runs[0].status, "running");
        assert!(running.sync_runs[0].completed_at.is_none());

        store
            .finish_sync(
                &interrupted,
                SyncRunStatus::BudgetExceeded,
                None,
                None,
                None,
            )
            .expect("finish interrupted sync");
        let completed = store
            .begin_sync("work-code", "work", 200, 4_096, 60)
            .expect("begin completed sync");
        store
            .finish_sync(
                &completed,
                SyncRunStatus::Succeeded,
                Some(12),
                Some(1_024),
                Some(2),
            )
            .expect("finish completed sync");

        let latest = store.stats().expect("latest stats");
        assert_eq!(latest.sync_runs.len(), 1);
        let run = &latest.sync_runs[0];
        assert_eq!(run.status, "succeeded");
        assert_eq!(run.documents, Some(12));
        assert_eq!(run.bytes, Some(1_024));
        assert_eq!(run.deleted, Some(2));
        assert_eq!(run.budget_documents, 200);
        assert_eq!(run.budget_bytes, 4_096);
        assert_eq!(run.budget_seconds, 60);
        assert!(run.completed_at.is_some());
    }

    #[test]
    fn sync_run_history_is_bounded_per_source() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        for _ in 0..(SYNC_RUNS_PER_SOURCE + 5) {
            let run = store
                .begin_sync("source", "project", 10, 1_024, 30)
                .expect("begin sync");
            store
                .finish_sync(&run, SyncRunStatus::Succeeded, Some(1), Some(10), Some(0))
                .expect("finish sync");
        }
        let connection = store.connection.lock().expect("store lock");
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sync_runs WHERE source='source' AND project='project'",
                [],
                |row| row.get(0),
            )
            .expect("history count");
        assert_eq!(count, i64::try_from(SYNC_RUNS_PER_SOURCE).unwrap());
    }

    #[test]
    fn sync_run_recovery_cancels_orphaned_running_runs() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .begin_sync("work-code", "work", 100, 2_048, 30)
            .expect("begin first run");
        store
            .begin_sync("personal", "home", 50, 1_024, 60)
            .expect("begin second run");

        assert_eq!(
            store.recover_interrupted_syncs().expect("recover runs"),
            2,
            "every orphaned running run is recovered"
        );
        assert_eq!(
            store.recover_interrupted_syncs().expect("recover again"),
            0,
            "recovery is idempotent"
        );

        let connection = store.connection.lock().expect("store lock");
        let rows = {
            let mut statement = connection
                .prepare(
                    "SELECT source,status,completed_at,documents,bytes,deleted,
                            budget_documents,budget_bytes,budget_seconds
                     FROM sync_runs ORDER BY started_at",
                )
                .expect("prepare sync runs");
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                })
                .expect("query sync runs")
                .collect::<rusqlite::Result<Vec<_>>>()
                .expect("collect sync runs")
        };
        drop(connection);

        let budgets: Vec<(String, (i64, i64, i64))> = vec![
            ("work-code".into(), (100, 2_048, 30)),
            ("personal".into(), (50, 1_024, 60)),
        ];
        assert_eq!(rows.len(), budgets.len());
        for (
            source,
            status,
            completed_at,
            documents,
            bytes,
            deleted,
            budget_documents,
            budget_bytes,
            budget_seconds,
        ) in rows
        {
            assert_eq!(status, "cancelled");
            assert!(
                completed_at.is_some(),
                "recovered run records a completion timestamp"
            );
            assert_eq!(documents, None, "outcome counters stay untouched");
            assert_eq!(bytes, None, "outcome counters stay untouched");
            assert_eq!(deleted, None, "outcome counters stay untouched");
            assert_eq!(
                (budget_documents, budget_bytes, budget_seconds),
                budgets
                    .iter()
                    .find(|(expected, _)| expected == &source)
                    .expect("matching budget")
                    .1,
                "configured budgets survive recovery"
            );
        }
    }

    #[test]
    fn sync_run_recovery_preserves_completed_outcomes() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let completed = store
            .begin_sync("work-code", "work", 200, 4_096, 60)
            .expect("begin completed run");
        store
            .finish_sync(
                &completed,
                SyncRunStatus::Succeeded,
                Some(12),
                Some(1_024),
                Some(2),
            )
            .expect("finish completed run");
        let interrupted = store
            .begin_sync("work-code", "work", 100, 2_048, 30)
            .expect("begin interrupted run");

        let completed_at_before: String = {
            let connection = store.connection.lock().expect("store lock");
            connection
                .query_row(
                    "SELECT completed_at FROM sync_runs WHERE id=?1",
                    [&completed],
                    |row| row.get(0),
                )
                .expect("completed run timestamp")
        };

        assert_eq!(store.recover_interrupted_syncs().expect("recover runs"), 1);

        let (status, completed_at, documents, bytes, deleted): (
            String,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        ) = {
            let connection = store.connection.lock().expect("store lock");
            connection
                .query_row(
                    "SELECT status,completed_at,documents,bytes,deleted FROM sync_runs WHERE id=?1",
                    [&completed],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .expect("completed run row")
        };
        assert_eq!(status, "succeeded");
        assert_eq!(
            completed_at.as_deref(),
            Some(completed_at_before.as_str()),
            "completed runs are not rewritten by recovery"
        );
        assert_eq!(
            (documents, bytes, deleted),
            (Some(12), Some(1_024), Some(2)),
            "completed outcomes survive recovery"
        );

        let (status, completed_at): (String, Option<String>) = {
            let connection = store.connection.lock().expect("store lock");
            connection
                .query_row(
                    "SELECT status,completed_at FROM sync_runs WHERE id=?1",
                    [&interrupted],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("interrupted run row")
        };
        assert_eq!(status, "cancelled");
        assert!(completed_at.is_some());
        assert!(
            store
                .finish_sync(&interrupted, SyncRunStatus::Succeeded, None, None, None)
                .is_err(),
            "a recovered run cannot be completed again"
        );
    }

    #[test]
    fn sync_run_recovery_is_metadata_only_and_preserves_retention() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .upsert(
                &document("kept", "kept"),
                &[("kept".into(), vec![1.0, 0.0])],
            )
            .expect("insert document");
        let documents_before = store.stats().expect("stats before").documents;

        let mut recovered = 0;
        for _ in 0..5 {
            store
                .begin_sync("source", "project", 10, 1_024, 30)
                .expect("begin sync");
            recovered += 1;
        }
        assert_eq!(
            store.recover_interrupted_syncs().expect("recover runs"),
            recovered
        );

        let after_recovery = store.stats().expect("stats after");
        assert_eq!(
            after_recovery.documents, documents_before,
            "recovery never touches document data"
        );
        assert_eq!(after_recovery.sync_runs.len(), 1);
        assert_eq!(after_recovery.sync_runs[0].status, "cancelled");
        assert!(after_recovery.sync_runs[0].completed_at.is_some());

        for _ in 0..(SYNC_RUNS_PER_SOURCE + 5) {
            let run = store
                .begin_sync("source", "project", 10, 1_024, 30)
                .expect("begin sync");
            store
                .finish_sync(&run, SyncRunStatus::Succeeded, Some(1), Some(10), Some(0))
                .expect("finish sync");
        }
        let connection = store.connection.lock().expect("store lock");
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sync_runs WHERE source='source' AND project='project'",
                [],
                |row| row.get(0),
            )
            .expect("history count");
        assert_eq!(
            count,
            i64::try_from(SYNC_RUNS_PER_SOURCE).unwrap(),
            "recovered runs still count toward the per-source retention bound"
        );
    }

    #[test]
    fn semantic_candidates_load_only_requested_chunks() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .upsert(
                &document("near", "nearest"),
                &[("nearest".into(), vec![1.0, 0.0])],
            )
            .expect("insert nearest");
        store
            .upsert(
                &document("far", "farthest"),
                &[("farthest".into(), vec![0.0, 1.0])],
            )
            .expect("insert farthest");

        let candidates = store
            .semantic_ids(&[1.0, 0.0], Some("demo"), Some("test"), 1)
            .expect("semantic candidates");
        assert_eq!(candidates.len(), 1);
        assert!((candidates[0].1 - 1.0).abs() < f32::EPSILON);
        let chunks = store
            .chunks_by_ids(&[candidates[0].0.clone()])
            .expect("candidate chunks");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].source_id, "near");
        assert!(
            store
                .chunks_by_ids(&[])
                .expect("empty candidates")
                .is_empty()
        );
    }

    #[test]
    fn embedding_cache_tracks_reuse_by_fingerprint_and_content() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .cache_embedding("model-a", "same text", &[0.25, 0.75])
            .expect("cache embedding");

        assert_eq!(
            store
                .cached_embedding("model-a", "same text")
                .expect("cache read"),
            Some(vec![0.25, 0.75])
        );
        assert_eq!(
            store
                .cached_embedding("model-b", "same text")
                .expect("other model"),
            None
        );
        let stats = store.stats().expect("stats");
        assert_eq!(stats.embedding_cache_entries, 1);
        assert_eq!(stats.embedding_cache_hits, 1);
    }

    #[test]
    fn fingerprint_mismatch_explains_the_required_generation_change() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .ensure_fingerprint("model-a:16")
            .expect("initial fingerprint");

        let error = store
            .ensure_fingerprint("model-b:32")
            .expect_err("mismatched generation must fail closed");
        let message = error.to_string();
        assert!(message.contains("model-a:16"));
        assert!(message.contains("model-b:32"));
        assert!(message.contains("rebuild into a new generation"));
    }

    #[test]
    fn embedding_generation_migration_updates_meta_and_invalidates_derived_caches() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .ensure_fingerprint("legacy-endpoint:model-a:16")
            .expect("initial fingerprint");
        store
            .cache_embedding("legacy-endpoint:model-a:16", "same text", &[0.25, 0.75])
            .expect("cache embedding");
        store
            .cache_query("cached-query", "{\"ok\":true}", 10)
            .expect("cache query");

        store
            .migrate_embedding_fingerprint(
                "legacy-endpoint:model-a:16",
                "openai:http://127.0.0.1:6999/v1:model-a:16",
            )
            .expect("migrate generation");

        let stats = store.stats().expect("stats");
        assert_eq!(
            stats.embedding_fingerprint.as_deref(),
            Some("openai:http://127.0.0.1:6999/v1:model-a:16")
        );
        assert_eq!(stats.embedding_cache_entries, 0);
        assert_eq!(stats.query_cache_entries, 0);
        store
            .ensure_fingerprint("openai:http://127.0.0.1:6999/v1:model-a:16")
            .expect("new generation matches");
    }

    #[test]
    fn embedding_generation_migration_requires_an_exact_current_generation() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .ensure_fingerprint("current:model-a:16")
            .expect("initial fingerprint");

        let error = store
            .migrate_embedding_fingerprint("stale:model-a:16", "new:model-a:16")
            .expect_err("stale source generation must fail closed");
        assert!(error.to_string().contains("expected: stale:model-a:16"));
        assert_eq!(
            store
                .stats()
                .expect("stats")
                .embedding_fingerprint
                .as_deref(),
            Some("current:model-a:16")
        );
    }

    #[test]
    fn embedding_rebuild_stages_vectors_and_commits_atomically() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .ensure_fingerprint("old:model:2")
            .expect("initial fingerprint");
        store
            .upsert(
                &document("one", "first chunk"),
                &[("first chunk".into(), vec![1.0, 0.0])],
            )
            .expect("insert first");
        store
            .upsert(
                &document("two", "second chunk"),
                &[("second chunk".into(), vec![0.0, 1.0])],
            )
            .expect("insert second");
        store
            .cache_query("cached", "{\"ok\":true}", 10)
            .expect("cache query");

        assert_eq!(
            store
                .begin_embedding_rebuild("old:model:2", "new:model:2")
                .expect("begin rebuild"),
            2
        );
        let page = store
            .embedding_rebuild_chunks(None, 10)
            .expect("read rebuild page");
        assert_eq!(page.len(), 2);
        store
            .stage_embedding_rebuild(&[(page[0].0.clone(), vec![0.5, 0.5])])
            .expect("stage first");
        let incomplete = store
            .commit_embedding_rebuild("old:model:2", "new:model:2")
            .expect_err("incomplete rebuild must not commit");
        assert!(incomplete.to_string().contains("staged 1 of 2"));
        assert_eq!(
            store
                .stats()
                .expect("stats after incomplete rebuild")
                .embedding_fingerprint
                .as_deref(),
            Some("old:model:2")
        );
        assert_eq!(
            store.all_chunks(None, None).expect("live chunks")[0].embedding,
            vec![1.0, 0.0]
        );

        store
            .stage_embedding_rebuild(&[(page[1].0.clone(), vec![0.25, 0.75])])
            .expect("stage second");
        assert_eq!(
            store
                .commit_embedding_rebuild("old:model:2", "new:model:2")
                .expect("commit rebuild"),
            2
        );
        let stats = store.stats().expect("final stats");
        assert_eq!(stats.embedding_fingerprint.as_deref(), Some("new:model:2"));
        assert_eq!(stats.query_cache_entries, 0);
        let vectors = store
            .all_chunks(None, None)
            .expect("rebuilt chunks")
            .into_iter()
            .map(|chunk| chunk.embedding)
            .collect::<Vec<_>>();
        assert!(vectors.contains(&vec![0.5, 0.5]));
        assert!(vectors.contains(&vec![0.25, 0.75]));
    }

    #[test]
    fn optional_cache_writes_do_not_block_readers_during_external_writes() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("store.sqlite3");
        let store = Store::open(&path).expect("store");
        store
            .cache_embedding("model", "cached text", &[0.25, 0.75])
            .expect("seed cache");
        let blocker = Connection::open(&path).expect("blocking connection");
        blocker
            .execute_batch("PRAGMA journal_mode=WAL; BEGIN IMMEDIATE;")
            .expect("hold writer lock");

        assert_eq!(
            store
                .cached_embedding("model", "cached text")
                .expect("cache read"),
            Some(vec![0.25, 0.75])
        );
        assert!(
            !store
                .cache_embedding_if_available("model", "new text", &[0.5, 0.5])
                .expect("best-effort cache write")
        );

        blocker.execute_batch("ROLLBACK").expect("release lock");
        assert!(
            store
                .cache_embedding_if_available("model", "new text", &[0.5, 0.5])
                .expect("cache write after lock")
        );
    }

    #[test]
    fn stats_uses_a_read_connection_when_the_primary_connection_is_busy() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let _primary_connection_guard = store.connection.lock().expect("store lock");

        let stats = store
            .stats()
            .expect("stats must not wait on the primary mutex");
        assert_eq!(stats.documents, 0);
        assert_eq!(stats.chunks, 0);
    }

    #[test]
    fn migrates_legacy_json_embeddings_to_compact_blobs() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("store.sqlite3");
        let store = Store::open(&path).expect("open store");
        store
            .upsert(
                &document("legacy", "legacy content"),
                &[("legacy content".into(), vec![0.25, 0.75])],
            )
            .expect("insert chunk");
        store
            .cache_embedding("model", "legacy content", &[0.25, 0.75])
            .expect("cache embedding");
        drop(store);
        let connection = Connection::open(&path).expect("open raw database");
        connection
            .execute(
                "UPDATE chunks SET embedding_json='[0.25,0.75]',embedding_blob=NULL",
                [],
            )
            .expect("legacy chunk");
        connection
            .execute(
                "UPDATE embedding_cache
                 SET embedding_json='[0.25,0.75]',embedding_blob=NULL",
                [],
            )
            .expect("legacy cache");
        drop(connection);

        let migrated = Store::open(&path).expect("migrate store");
        assert_eq!(
            migrated.all_chunks(None, None).expect("chunks")[0].embedding,
            vec![0.25, 0.75]
        );
        assert_eq!(
            migrated
                .cached_embedding("model", "legacy content")
                .expect("cache"),
            Some(vec![0.25, 0.75])
        );
        let connection = Connection::open(&path).expect("inspect database");
        for table in ["chunks", "embedding_cache"] {
            let blob_rows: i64 = connection
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {table}
                         WHERE embedding_blob IS NOT NULL AND embedding_json='[]'"
                    ),
                    [],
                    |row| row.get(0),
                )
                .expect("blob count");
            assert_eq!(blob_rows, 1);
        }
    }

    #[test]
    fn embedding_cache_prunes_to_configured_bound() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .cache_embedding("model", "first", &[1.0])
            .expect("cache first");
        store
            .cache_embedding("model", "second", &[2.0])
            .expect("cache second");

        assert_eq!(store.prune_embedding_cache(1).expect("prune"), 1);
        assert_eq!(store.stats().expect("stats").embedding_cache_entries, 1);
    }

    #[cfg(unix)]
    #[test]
    fn opening_a_symlinked_database_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().expect("temporary directory");
        let target = directory.path().join("target.sqlite3");
        Store::open(&target).expect("target database");
        let linked = directory.path().join("linked.sqlite3");
        symlink(&target, &linked).expect("database symlink");

        let error = Store::open(&linked)
            .err()
            .expect("symlinked database must fail");
        assert!(error.to_string().contains("symlinked database path"));
    }

    #[cfg(unix)]
    #[test]
    fn opening_a_database_with_a_symlinked_sidecar_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().expect("temporary directory");
        let database = directory.path().join("store.sqlite3");
        Store::open(&database).expect("database");
        let external = directory.path().join("external-wal");
        std::fs::write(&external, b"not a sqlite wal").expect("external sidecar");
        let wal = PathBuf::from(format!("{}-wal", database.display()));
        symlink(&external, &wal).expect("sidecar symlink");

        let error = Store::open(&database)
            .err()
            .expect("symlinked sidecar must fail");
        assert!(error.to_string().contains("symlinked database path"));
    }

    #[test]
    fn backup_is_consistent_and_refuses_overwrite() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .upsert(
                &document("one", "recoverable"),
                &[("recoverable".into(), vec![1.0])],
            )
            .expect("insert");
        let backup = directory.path().join("backups/brain.sqlite3");

        store.backup(&backup).expect("backup");
        Store::verify(&backup).expect("verify");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                std::fs::metadata(&backup)
                    .expect("backup metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(directory.path().join("store.sqlite3"))
                    .expect("database metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let restored = Store::open(&backup).expect("open backup");
        assert_eq!(restored.stats().expect("stats").documents, 1);
        drop(restored);
        assert!(store.backup(&backup).is_err());

        let target = directory.path().join("target.sqlite3");
        let target_store = Store::open(&target).expect("open target");
        target_store
            .upsert(
                &document("old", "replace me"),
                &[("replace me".into(), vec![2.0])],
            )
            .expect("insert target");
        drop(target_store);
        let recovery = directory.path().join("backups/pre-restore.sqlite3");
        Store::restore(&target, &backup, Some(&recovery)).expect("restore");
        assert_eq!(
            Store::open(&target)
                .expect("restored")
                .stats()
                .expect("stats")
                .documents,
            1
        );
        assert_eq!(
            Store::open(&recovery)
                .expect("recovery")
                .all_chunks(None, None)
                .expect("recovery chunks")[0]
                .content,
            "replace me"
        );
    }

    #[cfg(unix)]
    #[test]
    fn backup_and_restore_reject_symlinked_paths() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().expect("temporary directory");
        let database = directory.path().join("store.sqlite3");
        let store = Store::open(&database).expect("open store");

        let backup_target = directory.path().join("outside.sqlite3");
        let backup_link = directory.path().join("backup.sqlite3");
        symlink(&backup_target, &backup_link).expect("backup symlink");
        let backup_error = store
            .backup(&backup_link)
            .expect_err("backup symlink must fail");
        assert!(backup_error.to_string().contains("symlinked database path"));

        let restore_target = directory.path().join("restore.sqlite3");
        let restore_link = directory.path().join("restore-link.sqlite3");
        symlink(&restore_target, &restore_link).expect("restore symlink");
        let restore_error =
            Store::restore(&restore_link, &database, None).expect_err("restore symlink must fail");
        assert!(
            restore_error
                .to_string()
                .contains("symlinked database path")
        );
    }

    #[test]
    fn native_memory_is_idempotent_acl_scoped_and_redactable() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let input = crate::memory::MemoryInput {
            kind: "preference".into(),
            project: "work".into(),
            title: "Review preference".into(),
            content: "Prefer concise release notes with explicit risks.".into(),
            source: "agent".into(),
            source_id: "session-1".into(),
            dedupe_key: Some("work:release-notes".into()),
            confidence: 0.9,
            importance: 0.8,
            acl: vec!["work".into()],
            provenance: serde_json::json!({"session":"session-1","evidence":["doc-1"]}),
            supersedes_id: None,
            valid_until: None,
        };
        let first = store.remember(&input).expect("remember");
        let revision = store.memory_revision().expect("memory revision");
        let second = store.remember(&input).expect("idempotent remember");
        assert_eq!(first.id, second.id);
        assert_eq!(store.memory_revision().expect("stable revision"), revision);
        assert_eq!(store.memory_stats().expect("stats").active, 1);
        assert_eq!(
            store
                .recall_memories(
                    "concise release notes",
                    Some("work"),
                    None,
                    10,
                    &["work".into()]
                )
                .expect("work recall")
                .len(),
            1
        );
        assert_eq!(
            store
                .recall_memories(
                    "what is my preference for concise release notes",
                    Some("work"),
                    None,
                    10,
                    &["work".into()]
                )
                .expect("natural-language fallback")
                .len(),
            1
        );
        assert!(
            store
                .recall_memories(
                    "concise release notes",
                    Some("work"),
                    None,
                    10,
                    &["personal".into()]
                )
                .expect("personal recall")
                .is_empty()
        );
        assert!(store.forget_memory(&first.id).expect("forget"));
        assert!(
            store
                .memory(&first.id)
                .expect("tombstone")
                .expect("memory")
                .content
                .is_empty()
        );
        assert!(
            store
                .recall_memories(
                    "concise release notes",
                    Some("work"),
                    None,
                    10,
                    &["work".into()]
                )
                .expect("redacted recall")
                .is_empty()
        );
    }

    #[test]
    fn scoped_memory_mutations_cannot_cross_acl_boundaries() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let personal = store
            .remember(&crate::memory::MemoryInput {
                kind: "preference".into(),
                project: "personal".into(),
                title: "Private preference".into(),
                content: "Keep private context private.".into(),
                source: "agent".into(),
                source_id: "private-session".into(),
                dedupe_key: Some("shared-retry-key".into()),
                confidence: 0.9,
                importance: 0.9,
                acl: vec!["personal".into()],
                provenance: serde_json::json!({"test":true}),
                supersedes_id: None,
                valid_until: None,
            })
            .expect("personal memory");

        let replacement = crate::memory::MemoryInput {
            kind: "preference".into(),
            project: "work".into(),
            title: "Cross-scope overwrite".into(),
            content: "This must not replace the personal memory.".into(),
            source: "agent".into(),
            source_id: "work-session".into(),
            dedupe_key: Some("shared-retry-key".into()),
            confidence: 0.9,
            importance: 0.9,
            acl: vec!["work".into()],
            provenance: serde_json::json!({"test":true}),
            supersedes_id: None,
            valid_until: None,
        };
        let error = store
            .remember_scoped(&replacement, &["work".into()], false)
            .expect_err("dedupe overwrite must be ACL-scoped");
        assert!(error.to_string().contains("outside principal visibility"));

        let superseding = crate::memory::MemoryInput {
            supersedes_id: Some(personal.id.clone()),
            dedupe_key: Some("work-replacement".into()),
            ..replacement
        };
        let error = store
            .remember_scoped(&superseding, &["work".into()], false)
            .expect_err("supersession must be ACL-scoped");
        assert!(error.to_string().contains("outside principal visibility"));

        let error = store
            .forget_memory_scoped(&personal.id, &["work".into()], false)
            .expect_err("forget must be ACL-scoped");
        assert!(error.to_string().contains("memory ACL denied"));
        assert_eq!(
            store
                .memory(&personal.id)
                .expect("read personal memory")
                .expect("personal memory exists")
                .status,
            "active"
        );
        assert!(
            store
                .export_memories(None, None, 100, &["work".into()])
                .expect("scoped export")
                .is_empty()
        );
        assert_eq!(
            store
                .export_memories(None, None, 100, &["*".into()])
                .expect("owner export")
                .len(),
            1
        );
    }

    #[test]
    fn expired_working_memory_is_not_recalled() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .remember(&crate::memory::MemoryInput {
                kind: "working".into(),
                project: "work".into(),
                title: "Expired task".into(),
                content: "This task is already complete.".into(),
                source: "agent".into(),
                source_id: String::new(),
                dedupe_key: None,
                confidence: 0.7,
                importance: 0.5,
                acl: vec![],
                provenance: serde_json::json!({"test":true}),
                supersedes_id: None,
                valid_until: Some("2000-01-01T00:00:00Z".into()),
            })
            .expect("expired memory");
        assert!(
            store
                .recall_memories("task complete", Some("work"), None, 10, &["work".into()])
                .expect("recall")
                .is_empty()
        );
        let stats = store.memory_stats().expect("memory stats");
        assert_eq!(stats.active, 0);
        assert_eq!(stats.expired, 1);
        assert_eq!(stats.total, 1);
        store
            .configure_memory_limit(1)
            .expect("configure memory limit");
        store
            .remember(&crate::memory::MemoryInput {
                kind: "working".into(),
                project: "work".into(),
                title: "Current task".into(),
                content: "This task is still active.".into(),
                source: "agent".into(),
                source_id: String::new(),
                dedupe_key: None,
                confidence: 0.7,
                importance: 0.5,
                acl: vec![],
                provenance: serde_json::json!({"test":true}),
                supersedes_id: None,
                valid_until: None,
            })
            .expect("expired records do not consume active capacity");
    }

    #[test]
    fn memory_recall_filters_acl_before_bounded_candidate_limit() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        for index in 0..8 {
            store
                .remember(&crate::memory::MemoryInput {
                    kind: "working".into(),
                    project: "private".into(),
                    title: format!("Shared task {index}"),
                    content: "shared task context".into(),
                    source: "agent".into(),
                    source_id: format!("private-{index}"),
                    dedupe_key: None,
                    confidence: 1.0,
                    importance: 1.0,
                    acl: vec!["private".into()],
                    provenance: serde_json::json!({"test":true}),
                    supersedes_id: None,
                    valid_until: None,
                })
                .expect("private memory");
        }
        let visible = store
            .remember(&crate::memory::MemoryInput {
                kind: "working".into(),
                project: "work".into(),
                title: "Shared task for work".into(),
                content: "shared task context".into(),
                source: "agent".into(),
                source_id: "work-1".into(),
                dedupe_key: None,
                confidence: 0.7,
                importance: 0.5,
                acl: vec!["work".into()],
                provenance: serde_json::json!({"test":true}),
                supersedes_id: None,
                valid_until: None,
            })
            .expect("visible memory");
        let results = store
            .recall_memories("shared task context", None, None, 1, &["work".into()])
            .expect("scoped recall");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].memory.id, visible.id);
        let exported = store
            .export_memories(None, None, 1, &["work".into()])
            .expect("scoped export");
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].id, visible.id);
    }

    #[test]
    fn native_memory_supersession_deactivates_previous_fact_atomically() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let previous = store
            .remember(&crate::memory::MemoryInput {
                kind: "semantic".into(),
                project: "personal".into(),
                title: "Current editor".into(),
                content: "The current editor is Vim.".into(),
                source: "agent".into(),
                source_id: "session-1".into(),
                dedupe_key: Some("personal:editor".into()),
                confidence: 0.7,
                importance: 0.5,
                acl: vec![],
                provenance: serde_json::json!({"session":"session-1"}),
                supersedes_id: None,
                valid_until: None,
            })
            .expect("previous");
        let replacement = store
            .remember(&crate::memory::MemoryInput {
                kind: "semantic".into(),
                project: "personal".into(),
                title: "Current editor".into(),
                content: "The current editor is Helix.".into(),
                source: "agent".into(),
                source_id: "session-2".into(),
                dedupe_key: Some("personal:editor-v2".into()),
                confidence: 0.95,
                importance: 0.8,
                acl: vec![],
                provenance: serde_json::json!({"session":"session-2"}),
                supersedes_id: Some(previous.id.clone()),
                valid_until: None,
            })
            .expect("replacement");
        assert_eq!(store.memory_stats().expect("stats").active, 1);
        assert_eq!(store.memory_stats().expect("stats").superseded, 1);
        let results = store
            .recall_memories(
                "current editor",
                Some("personal"),
                None,
                10,
                &["personal".into()],
            )
            .expect("recall");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].memory.id, replacement.id);
    }

    #[test]
    fn native_memory_enforces_configured_active_limit_without_blocking_replacement() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store.configure_memory_limit(1).expect("configure limit");
        let first = store
            .remember(&crate::memory::MemoryInput {
                kind: "working".into(),
                project: "work".into(),
                title: "Current task".into(),
                content: "Ship the native memory contract.".into(),
                source: "agent".into(),
                source_id: "session-1".into(),
                dedupe_key: None,
                confidence: 0.7,
                importance: 0.5,
                acl: vec![],
                provenance: serde_json::json!({"session":"session-1"}),
                supersedes_id: None,
                valid_until: None,
            })
            .expect("first memory");
        let error = store
            .remember(&crate::memory::MemoryInput {
                kind: "working".into(),
                project: "work".into(),
                title: "Another task".into(),
                content: "This must wait.".into(),
                source: "agent".into(),
                source_id: "session-2".into(),
                dedupe_key: None,
                confidence: 0.7,
                importance: 0.5,
                acl: vec![],
                provenance: serde_json::json!({"session":"session-2"}),
                supersedes_id: None,
                valid_until: None,
            })
            .expect_err("active limit must reject a second memory");
        assert!(error.to_string().contains("active memory limit reached"));
        let replacement = store
            .remember(&crate::memory::MemoryInput {
                kind: "working".into(),
                project: "work".into(),
                title: "Current task".into(),
                content: "The native memory contract shipped.".into(),
                source: "agent".into(),
                source_id: "session-3".into(),
                dedupe_key: Some("work:current-task".into()),
                confidence: 0.95,
                importance: 0.8,
                acl: vec![],
                provenance: serde_json::json!({"session":"session-3"}),
                supersedes_id: Some(first.id.clone()),
                valid_until: None,
            })
            .expect("replacement stays within limit");
        assert_ne!(replacement.id, first.id);
        assert_eq!(store.memory_stats().expect("stats").active, 1);
    }

    #[test]
    fn expired_supersession_target_cannot_bypass_active_limit() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store.configure_memory_limit(1).expect("configure limit");
        let expired = store
            .remember(&crate::memory::MemoryInput {
                kind: "working".into(),
                project: "work".into(),
                title: "Expired task".into(),
                content: "This task is already complete.".into(),
                source: "agent".into(),
                source_id: "session-expired".into(),
                dedupe_key: None,
                confidence: 0.7,
                importance: 0.5,
                acl: vec![],
                provenance: serde_json::json!({"session":"session-expired"}),
                supersedes_id: None,
                valid_until: Some("2000-01-01T00:00:00Z".into()),
            })
            .expect("expired memory");
        store
            .remember(&crate::memory::MemoryInput {
                kind: "working".into(),
                project: "work".into(),
                title: "Current task".into(),
                content: "Keep the active memory cap.".into(),
                source: "agent".into(),
                source_id: "session-current".into(),
                dedupe_key: None,
                confidence: 0.7,
                importance: 0.5,
                acl: vec![],
                provenance: serde_json::json!({"session":"session-current"}),
                supersedes_id: None,
                valid_until: None,
            })
            .expect("current memory");
        let error = store
            .remember(&crate::memory::MemoryInput {
                kind: "working".into(),
                project: "work".into(),
                title: "Overflow task".into(),
                content: "This must wait.".into(),
                source: "agent".into(),
                source_id: "session-overflow".into(),
                dedupe_key: None,
                confidence: 0.7,
                importance: 0.5,
                acl: vec![],
                provenance: serde_json::json!({"session":"session-overflow"}),
                supersedes_id: Some(expired.id),
                valid_until: None,
            })
            .expect_err("expired supersession must not bypass the active limit");
        assert!(error.to_string().contains("active memory limit reached"));
    }
}
