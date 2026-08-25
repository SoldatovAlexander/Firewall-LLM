"""Audit log: append-only record of requests/responses with PII redaction."""

from __future__ import annotations

import json
import sqlite3
import threading
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from lightanon.rag import TextSanitizer

from fwllm.config import AuditConfig


class AuditLog:
    def __init__(self, config: AuditConfig):
        self._config = config
        self._lock = threading.Lock()
        self._conn = sqlite3.connect(config.db_path, check_same_thread=False)
        self._conn.execute(
            """
            CREATE TABLE IF NOT EXISTS audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts TEXT NOT NULL,
                client TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                code TEXT NOT NULL,
                prompt_tokens INTEGER NOT NULL DEFAULT 0,
                completion_tokens INTEGER NOT NULL DEFAULT 0,
                messages TEXT NOT NULL,
                response_text TEXT NOT NULL
            )
            """
        )
        self._conn.commit()

    @property
    def enabled(self) -> bool:
        return self._config.enabled

    def _redact(self, text: str, sanitizer: TextSanitizer) -> str:
        if not self._config.dlp_redact:
            return text
        redacted: str = sanitizer.sanitize(text)
        return redacted

    def write(
        self,
        *,
        client: str,
        provider: str,
        model: str,
        code: str,
        prompt_tokens: int,
        completion_tokens: int,
        messages: list[dict[str, Any]],
        response_text: str,
    ) -> None:
        sanitizer = TextSanitizer(profile="ru_152")
        redacted_messages = [
            {
                **message,
                "content": (
                    self._redact(message["content"], sanitizer)
                    if isinstance(message.get("content"), str)
                    else message.get("content")
                ),
            }
            for message in messages
        ]
        with self._lock:
            self._conn.execute(
                """
                INSERT INTO audit (ts, client, provider, model, code,
                                   prompt_tokens, completion_tokens,
                                   messages, response_text)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    datetime.now(UTC).isoformat(),
                    client,
                    provider,
                    model,
                    code,
                    prompt_tokens,
                    completion_tokens,
                    json.dumps(redacted_messages),
                    self._redact(response_text, sanitizer),
                ),
            )
            self._conn.commit()

    def search(
        self,
        *,
        client: str | None = None,
        code: str | None = None,
        limit: int = 100,
    ) -> list[dict[str, Any]]:
        query = (
            "SELECT ts, client, provider, model, code, "
            "prompt_tokens, completion_tokens, messages, response_text FROM audit"
        )
        conditions: list[str] = []
        params: list[str] = []
        if client is not None:
            conditions.append("client = ?")
            params.append(client)
        if code is not None:
            conditions.append("code = ?")
            params.append(code)
        if conditions:
            query += " WHERE " + " AND ".join(conditions)
        query += " ORDER BY id DESC LIMIT ?"
        params.append(str(limit))
        cursor = self._conn.execute(query, params)
        rows = cursor.fetchall()
        return [
            {
                "ts": row[0],
                "client": row[1],
                "provider": row[2],
                "model": row[3],
                "code": row[4],
                "prompt_tokens": row[5],
                "completion_tokens": row[6],
                "messages": json.loads(row[7]),
                "response_text": row[8],
            }
            for row in rows
        ]

    def close(self) -> None:
        with self._lock:
            self._conn.close()


def ensure_parent(db_path: str) -> None:
    Path(db_path).parent.mkdir(parents=True, exist_ok=True)
