use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::model::{Document, StoredChunk};

#[derive(Clone)]
pub struct Store {
    connection: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
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
               ordinal INTEGER NOT NULL, content TEXT NOT NULL, embedding_json TEXT NOT NULL);
             CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
               chunk_id UNINDEXED, title, content, tokenize='unicode61');
             CREATE INDEX IF NOT EXISTS idx_documents_scope ON documents(project, source);",
        )?;
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
        let hash = hex_digest(document.content.as_bytes());
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
                "INSERT INTO chunks(id,document_id,ordinal,content,embedding_json) VALUES(?1,?2,?3,?4,?5)",
                params![chunk_id, id, ordinal as i64, content, serde_json::to_string(embedding)?],
            )?;
            transaction.execute(
                "INSERT INTO chunks_fts(chunk_id,title,content) VALUES(?1,?2,?3)",
                params![chunk_id, document.title, content],
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    pub fn all_chunks(
        &self,
        project: Option<&str>,
        source: Option<&str>,
    ) -> Result<Vec<StoredChunk>> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(
            "SELECT c.id,d.source,d.source_id,d.title,d.uri,c.content,c.embedding_json,d.updated_at
             FROM chunks c JOIN documents d ON d.id=c.document_id
             WHERE (?1 IS NULL OR d.project=?1) AND (?2 IS NULL OR d.source=?2)",
        )?;
        let rows = statement.query_map(params![project, source], row_to_chunk)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn lexical_ids(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let connection = self.connection.lock().expect("store lock poisoned");
        let mut statement = connection.prepare(
            "SELECT chunk_id FROM chunks_fts WHERE chunks_fts MATCH ?1 ORDER BY bm25(chunks_fts) LIMIT ?2",
        )?;
        let safe_query = query
            .split_whitespace()
            .map(|token| format!("\"{}\"", token.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR ");
        let rows = statement.query_map(params![safe_query, limit as i64], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}

fn row_to_chunk(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredChunk> {
    let embedding: String = row.get(6)?;
    let updated_at: String = row.get(7)?;
    Ok(StoredChunk {
        id: row.get(0)?,
        source: row.get(1)?,
        source_id: row.get(2)?,
        title: row.get(3)?,
        uri: row.get(4)?,
        content: row.get(5)?,
        embedding: serde_json::from_str(&embedding).unwrap_or_default(),
        updated_at: DateTime::parse_from_rfc3339(&updated_at)
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
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
