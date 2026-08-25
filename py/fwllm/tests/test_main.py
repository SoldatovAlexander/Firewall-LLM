"""Entrypoint bootstrap tests."""

import textwrap

import pytest

from fwllm.main import create_app_from_env


def _write_cfg(tmp_path):
    cfg = tmp_path / "fwllm.yaml"
    cfg.write_text(
        textwrap.dedent(
            """
            providers:
              p:
                base_url: https://x.example/v1
                api_key_env: X_API_KEY
            """
        ),
        encoding="utf-8",
    )
    return cfg


def test_missing_config_env_raises(monkeypatch):
    monkeypatch.delenv("FWLLM_CONFIG", raising=False)
    with pytest.raises(RuntimeError, match="FWLLM_CONFIG"):
        create_app_from_env()


def test_bootstrap_from_env(tmp_path, monkeypatch):
    monkeypatch.setenv("X_API_KEY", "k")
    monkeypatch.setenv("FWLLM_CONFIG", str(_write_cfg(tmp_path)))
    monkeypatch.setenv("FWLLM_CLIENT_TOKENS", "tok1:alice,tok2:bob")
    app = create_app_from_env()
    assert app.state.clients == {"tok1": "alice", "tok2": "bob"}


def test_config_loads_client_tokens_from_env(tmp_path, monkeypatch):
    monkeypatch.setenv("X_API_KEY", "k")
    monkeypatch.setenv("FWLLM_CLIENT_TOKENS", "secret:carol, other:dave")
    from fwllm.config import load_config

    cfg = load_config(_write_cfg(tmp_path))
    assert cfg.clients == {"secret": "carol", "other": "dave"}
