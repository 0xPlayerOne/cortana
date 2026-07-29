#!/usr/bin/env python3
"""Stream legacy Chroma collections as Cortana pre-embedded JSONL records."""

from __future__ import annotations

import argparse
import contextlib
import datetime as dt
import json
import sys
from collections.abc import Iterator
from pathlib import Path
from typing import Any

DEFAULT_MODEL = "Qwen/Qwen3-Embedding-0.6B"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--chroma-dir", type=Path, required=True)
    parser.add_argument("--collection", action="append", choices=["code", "second-brain"])
    parser.add_argument("--developer-root", type=Path, default=Path.home() / "Developer")
    parser.add_argument(
        "--brain-root", type=Path, default=Path.home() / "Developer" / "second-brain"
    )
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--batch-size", type=int, default=256)
    return parser.parse_args()


def records(
    collection: Any,
    collection_name: str,
    developer_root: Path,
    brain_root: Path,
    model: str,
    batch_size: int,
) -> Iterator[dict[str, Any]]:
    fingerprint = f"{model}:1024"
    exported_at = dt.datetime.now(dt.UTC).isoformat()
    total = collection.count()
    for offset in range(0, total, batch_size):
        batch = collection.get(
            limit=batch_size,
            offset=offset,
            include=["documents", "metadatas", "embeddings"],
        )
        embeddings = batch["embeddings"]
        if embeddings is None:
            raise RuntimeError(f"collection {collection_name} returned no embeddings")
        for record_id, content, metadata, embedding in zip(
            batch["ids"],
            batch["documents"],
            batch["metadatas"],
            embeddings,
            strict=True,
        ):
            if not content or not str(content).strip():
                continue
            clean_metadata = dict(metadata or {})
            if collection_name == "code":
                repo = str(clean_metadata.get("repo") or "legacy-code")
                relative_path = str(clean_metadata.get("path") or record_id)
                path = developer_root / repo / relative_path
                source = "legacy-code"
                project = repo
                title = f"{repo}/{relative_path}"
            else:
                relative_path = str(clean_metadata.get("source_file") or record_id)
                path = brain_root / relative_path
                source = "legacy-second-brain"
                project = "second-brain"
                title = relative_path
                section = clean_metadata.get("section")
                if section:
                    title = f"{relative_path} — {section}"
            updated_at = exported_at
            with contextlib.suppress(OSError):
                updated_at = dt.datetime.fromtimestamp(path.stat().st_mtime, dt.UTC).isoformat()
            clean_metadata.update(
                {
                    "legacy_chroma_collection": collection_name,
                    "legacy_chroma_id": str(record_id),
                }
            )
            vector = embedding.tolist() if hasattr(embedding, "tolist") else list(embedding)
            yield {
                "embedding_fingerprint": fingerprint,
                "document": {
                    "source": source,
                    "source_id": str(record_id),
                    "title": title,
                    "content": str(content),
                    "uri": path.resolve().as_uri(),
                    "updated_at": updated_at,
                    "project": project,
                    "acl": [],
                    "metadata": clean_metadata,
                },
                "chunks": [{"content": str(content), "embedding": vector}],
            }
        print(
            f"exported {collection_name}: {min(offset + batch_size, total)}/{total}",
            file=sys.stderr,
        )


def main() -> int:
    args = parse_args()
    if args.batch_size < 1:
        raise ValueError("--batch-size must be positive")
    try:
        import chromadb
        from chromadb.config import Settings
    except ImportError as error:
        raise RuntimeError("run this exporter with the legacy Chroma environment") from error
    client = chromadb.PersistentClient(
        path=str(args.chroma_dir.expanduser()),
        settings=Settings(anonymized_telemetry=False),
    )
    names = args.collection or ["code", "second-brain"]
    for name in names:
        collection = client.get_collection(name)
        for record in records(
            collection,
            name,
            args.developer_root.expanduser(),
            args.brain_root.expanduser(),
            args.model,
            args.batch_size,
        ):
            print(json.dumps(record, ensure_ascii=False, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
