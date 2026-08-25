"""Idempotent import of dashboards and datasource into an existing Grafana."""

from __future__ import annotations

import argparse
import base64
import json
import sys
from pathlib import Path
from typing import Any

import httpx

DATASOURCE_UID = "fwllm-prometheus"


def _headers(auth: str | None) -> dict[str, str]:
    headers = {"Content-Type": "application/json"}
    if auth:
        headers["Authorization"] = auth
    return headers


def _auth(token: str | None, basic: str | None) -> str | None:
    if token:
        return f"Bearer {token}"
    if basic:
        encoded = base64.b64encode(basic.encode()).decode()
        return f"Basic {encoded}"
    return None


def ensure_datasource(client: httpx.Client, auth: str | None, url: str) -> bool:
    """Create the Prometheus datasource if missing. Returns True when created."""
    response = client.get(
        f"/api/datasources/uid/{DATASOURCE_UID}", headers=_headers(auth)
    )
    if response.status_code == 200:
        return False
    created = client.post(
        "/api/datasources",
        headers=_headers(auth),
        json={
            "name": DATASOURCE_UID,
            "uid": DATASOURCE_UID,
            "type": "prometheus",
            "url": url,
            "access": "proxy",
            "isDefault": False,
        },
    )
    created.raise_for_status()
    return True


def import_dashboard(
    client: httpx.Client,
    auth: str | None,
    dashboard: dict[str, Any],
    folder_uid: str | None = None,
) -> dict[str, Any]:
    """Import a dashboard; updates in place when the UID already exists."""
    payload: dict[str, Any] = {
        "dashboard": {**dashboard, "id": None},
        "overwrite": True,
        "message": "fwllm automated import",
    }
    if folder_uid:
        payload["folderUid"] = folder_uid
    response = client.post("/api/dashboards/db", headers=_headers(auth), json=payload)
    response.raise_for_status()
    result: dict[str, Any] = response.json()
    return result


def run_import(
    grafana_url: str,
    *,
    files: list[Path],
    token: str | None = None,
    basic: str | None = None,
    datasource_url: str = "http://prometheus:9090",
    folder_uid: str | None = None,
) -> list[tuple[str, dict[str, Any]]]:
    auth = _auth(token, basic)
    results: list[tuple[str, dict[str, Any]]] = []
    with httpx.Client(base_url=grafana_url.rstrip("/"), timeout=30.0) as client:
        ensure_datasource(client, auth, datasource_url)
        for path in files:
            dashboard = json.loads(path.read_text(encoding="utf-8"))
            result = import_dashboard(client, auth, dashboard, folder_uid=folder_uid)
            results.append((dashboard.get("title", path.name), result))
    return results


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Import fwllm dashboards to Grafana")
    parser.add_argument("--url", required=True, help="Grafana base URL")
    parser.add_argument("--token", help="Service account token (Bearer)")
    parser.add_argument("--basic", help="Basic credentials 'user:password'")
    parser.add_argument("--file", action="append", required=True, help="Dashboard JSON")
    parser.add_argument("--datasource-url", default="http://prometheus:9090")
    parser.add_argument("--folder-uid")
    args = parser.parse_args(argv)

    if not args.token and not args.basic:
        parser.error("provide --token or --basic")

    for title, result in run_import(
        args.url,
        files=[Path(p) for p in args.file],
        token=args.token,
        basic=args.basic,
        datasource_url=args.datasource_url,
        folder_uid=args.folder_uid,
    ):
        print(f"imported '{title}': {result}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
