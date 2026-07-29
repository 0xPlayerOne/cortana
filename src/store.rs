use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::{Document, StoredChunk};

#[derive(Clone)]
pub struct Store {
    connection: Arc<Mutex<Connection>>,
}

#[derive(Debug, Serialize)]
pub struct StoreStats {
    pub documents: i64,
    pub chunks: i64,
    pub embedding_fingerprint: Option<String>,
    pub embedding_cache_entries: i64,
    pub embedding_cache_hits: i64,
    pub sources: Vec<SourceStats>,
}

#[derive(Debug, Serialize)]
pub struct SourceStats {
    pub source: String,
    pub project: String,
    pub documents: i64,
    pub chunks: i64,
    pub latest_updated_at: Option<String>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS documents(
               id TEXT PRIMARY KEY, source TEXT NOT NULL, source_id TEXT NOT NULL,
               title TEXT NOT NULL, uri TEXT, content_hash TEXT NOT NULL,
               updated_at TEXT NOT NULL, project TEXT NOT NULL, acl_json TEXT NOT NULL,
               metadata_json TEXT NOT NULL, UNIQUE(source, source_id));
             CREATE TABLE IF NOT EXISTS chunks(
               id TEXT PRIMARY KEY, document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
               ordinal INTEGER NOT NULL, content TEXT NOT NULL, embedding_json TEXT NOT NULL,
               embedding_blob BLOB);
             CREATE TABLE IF NOT EXISTS embedding_cache(
               fingerprint TEXT NOT NULL, content_hash TEXT NOT NULL,
               embedding_json TEXT NOT NULL, embedding_blob BLOB, hits INTEGER NOT NULL DEFAULT 0,
               created_at TEXT NOT NULL, last_used_at TEXT NOT NULL,
               PRIMARY KEY(fingerprint,content_hash));
             CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
               chunk_id UNINDEXED, title, content, tokenize='unicode61');
             CREATE INDEX IF NOT EXISTS idx_documents_scope ON documents(project, source);",
        )?;
        migrate_embedding_blobs(&mut connection)?;
        secure_database_files(path)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
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
        if current.as_deref().is_some_and(|value| value != fingerprint) {
            bail!("embedding model differs from this index; rebuild into a new generation");
        }
        connection.execute(
            "INSERT OR IGNORE INTO meta(key,value) VALUES('embedding_fingerprint',?1)",
            [fingerprint],
        )?;
        Ok(())
    }

    pub fn upsert(&self, document: &Document, chunks: &[(String, Vec<f32>)]) -> Result<bool> {
        let id = stable_id(&document.source, &document.source_id);
        let hash = document_hash(document)?;
        let mut connection = self.connection.lock().expect("store lock poisoned");
        let previous: Option<String> = connection
            .query_row(
                "SELECT content_hash FROM documents WHERE id=?1",
                [&id],
                |row| row.get(0),
            )
            .optional()?;
        if previous.as_deref() == Some(&hash) {
            return Ok(false);
        }
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM chunks_fts WHERE chunk_id IN (SELECT id FROM chunks WHERE document_id=?1)",
            [&id],
        )?;
        transaction.execute("DELETE FROM chunks WHERE document_id=?1", [&id])?;
        transaction.execute(
            "INSERT INTO documents(id,source,source_id,title,uri,content_hash,updated_at,project,acl_json,metadata_json)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(id) DO UPDATE SET title=excluded.title,uri=excluded.uri,
             content_hash=excluded.content_hash,updated_at=excluded.updated_at,project=excluded.project,
             acl_json=excluded.acl_json,metadata_json=excluded.metadata_json",
            params![id, document.source, document.source_id, document.title, document.uri, hash,
                document.updated_at.to_rfc3339(), document.project,
                serde_json::to_string(&document.acl)?, document.metadata.to_string()],
        )?;
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
        transaction.commit()?;
        Ok(true)
    }

    pub fn needs_update(&self, document: &Document) -> Result<bool> {
        let id = stable_id(&document.source, &document.source_id);
        let hash = document_hash(document)?;
        let connection = self.connection.lock().expect("store lock poisoned");
        let previous: Option<String> = connection
            .query_row(
                "SELECT content_hash FROM documents WHERE id=?1",
                [&id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(previous.as_deref() != Some(&hash))
    }

    pub fn refresh_timestamp(&self, document: &Document) -> Result<()> {
        let id = stable_id(&document.source, &document.source_id);
        let connection = self.connection.lock().expect("store lock poisoned");
        connection.execute(
            "UPDATE documents SET updated_at=?2 WHERE id=?1",
            params![id, document.updated_at.to_rfc3339()],
        )?;
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
        transaction.commit()?;
        Ok(stale.len())
    }

    pub fn all_chunks(
        &self,
        project: Option<&str>,
        source: Option<&str>,
    ) -> Result<Vec<StoredChunk>> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(
            "SELECT c.id,d.source,d.source_id,d.title,d.uri,c.content,
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
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(
            "SELECT c.id,c.embedding_blob,c.embedding_json
             FROM chunks c JOIN documents d ON d.id=c.document_id
             WHERE (?1 IS NULL OR d.project=?1) AND (?2 IS NULL OR d.source=?2)",
        )?;
        let rows = statement.query_map(params![project, source], |row| {
            let id = row.get::<_, String>(0)?;
            let blob = row.get::<_, Option<Vec<u8>>>(1)?;
            let json = row.get::<_, String>(2)?;
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
            Ok((id, cosine(query_embedding, &embedding)))
        })?;
        let mut best = BinaryHeap::<Reverse<SemanticCandidate>>::with_capacity(limit);
        for row in rows {
            let (id, score) = row?;
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
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT c.id,d.source,d.source_id,d.title,d.uri,c.content,
                    c.embedding_blob,c.embedding_json,d.updated_at
             FROM chunks c JOIN documents d ON d.id=c.document_id
             WHERE c.id IN ({placeholders})"
        );
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(ids), row_to_chunk)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn lexical_ids(
        &self,
        query: &str,
        project: Option<&str>,
        source: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let safe_query = query
            .split_whitespace()
            .map(|token| token.replace('"', ""))
            .filter(|token| !token.is_empty())
            .map(|token| format!("\"{token}\""))
            .collect::<Vec<_>>()
            .join(" OR ");
        if safe_query.is_empty() {
            return Ok(Vec::new());
        }
        let mut statement = connection.prepare(
            "SELECT f.chunk_id FROM chunks_fts f
             JOIN chunks c ON c.id=f.chunk_id
             JOIN documents d ON d.id=c.document_id
             WHERE chunks_fts MATCH ?1
               AND (?2 IS NULL OR d.project=?2)
               AND (?3 IS NULL OR d.source=?3)
             ORDER BY bm25(chunks_fts) LIMIT ?4",
        )?;
        let rows = statement
            .query_map(params![safe_query, project, source, limit as i64], |row| {
                row.get(0)
            })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn stats(&self) -> Result<StoreStats> {
        let connection = self.connection.lock().expect("store lock poisoned");
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
        Ok(StoreStats {
            documents,
            chunks,
            embedding_fingerprint,
            embedding_cache_entries,
            embedding_cache_hits,
            sources,
        })
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
            connection.execute(
                "UPDATE embedding_cache SET hits=hits+1,last_used_at=?3
                 WHERE fingerprint=?1 AND content_hash=?2",
                params![fingerprint, hash, Utc::now().to_rfc3339()],
            )?;
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

fn verify_database(path: &Path) -> Result<()> {
    anyhow::ensure!(
        path.is_file(),
        "database does not exist: {}",
        path.display()
    );
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let result: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    anyhow::ensure!(result == "ok", "database integrity check failed: {result}");
    Ok(())
}

#[cfg(unix)]
fn secure_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to secure {}", path.display()))
}

#[cfg(not(unix))]
fn secure_file(_path: &Path) -> Result<()> {
    Ok(())
}

fn secure_database_files(database: &Path) -> Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let mut path = database.as_os_str().to_os_string();
        path.push(suffix);
        let path = PathBuf::from(path);
        if path.exists() {
            secure_file(&path)?;
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
    let embedding_blob: Option<Vec<u8>> = row.get(6)?;
    let embedding_json: String = row.get(7)?;
    let embedding = embedding_blob.map_or_else(
        || {
            serde_json::from_str(&embedding_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(7, Type::Text, Box::new(error))
            })
        },
        |blob| {
            decode_embedding(&blob).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(6, Type::Blob, error.into())
            })
        },
    )?;
    let updated_at: String = row.get(8)?;
    Ok(StoredChunk {
        id: row.get(0)?,
        source: row.get(1)?,
        source_id: row.get(2)?,
        title: row.get(3)?,
        uri: row.get(4)?,
        content: row.get(5)?,
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
    fn unchanged_documents_skip_work_and_reconciliation_deletes_stale_rows() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let first = document("one", "first");
        let second = document("two", "second");
        assert!(store.needs_update(&first).expect("check first"));
        store
            .upsert(&first, &[("first".into(), vec![1.0])])
            .expect("insert first");
        store
            .upsert(&second, &[("second".into(), vec![1.0])])
            .expect("insert second");
        assert!(!store.needs_update(&first).expect("check unchanged"));
        let mut refreshed = first.clone();
        refreshed.updated_at += chrono::Duration::days(1);
        store
            .refresh_timestamp(&refreshed)
            .expect("refresh timestamp");
        assert_eq!(
            store.all_chunks(None, None).expect("chunks")[0].updated_at,
            refreshed.updated_at
        );

        let deleted = store
            .reconcile("test", "demo", &["one".into()])
            .expect("reconcile");
        assert_eq!(deleted, 1);
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
}
