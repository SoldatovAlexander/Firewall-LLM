"""Tests for fwllm.config: YAML + env loading, validation, defaults."""

from textwrap import dedent

import pytest

from fwllm.config import ConfigError, load_config


def test_loads_yaml_with_defaults(tmp_path, monkeypatch):
    monkeypatch.setenv("OPENROUTER_API_KEY", "sk-test")
    cfg_file = tmp_path / "fwllm.yaml"
    cfg_file.write_text(
        dedent(
            """
            server:
              host: 0.0.0.0
              port: 8080
            providers:
              openrouter:
                base_url: https://openrouter.ai/api/v1
                api_key_env: OPENROUTER_API_KEY
            """
        ),
        encoding="utf-8",
    )
    cfg = load_config(cfg_file)
    assert cfg.server.port == 8080
    # defaults applied
    assert cfg.server.request_timeout_seconds == 120
    assert cfg.redis_url == "redis://localhost:6379/0"


def test_missing_required_provider_base_url_raises(tmp_path):
    cfg_file = tmp_path / "fwllm.yaml"
    cfg_file.write_text(
        "providers:\n  openrouter:\n    api_key_env: KEY\n", encoding="utf-8"
    )
    with pytest.raises(ConfigError, match="base_url"):
        load_config(cfg_file)


def test_empty_providers_raises(tmp_path):
    cfg_file = tmp_path / "fwllm.yaml"
    cfg_file.write_text("server:\n  port: 9000\n", encoding="utf-8")
    with pytest.raises(ConfigError, match="providers"):
        load_config(cfg_file)


def test_invalid_yaml_raises(tmp_path):
    cfg_file = tmp_path / "fwllm.yaml"
    cfg_file.write_text("server: [unclosed\n", encoding="utf-8")
    with pytest.raises(ConfigError, match="parse"):
        load_config(cfg_file)


def test_missing_file_raises(tmp_path):
    with pytest.raises(ConfigError, match="not found"):
        load_config(tmp_path / "nope.yaml")


def test_env_overrides_yaml(tmp_path, monkeypatch):
    monkeypatch.setenv("OPENROUTER_API_KEY", "sk-test")
    cfg_file = tmp_path / "fwllm.yaml"
    cfg_file.write_text(
        dedent(
            """
            server:
              host: 127.0.0.1
              port: 8080
            providers:
              openrouter:
                base_url: https://openrouter.ai/api/v1
                api_key_env: OPENROUTER_API_KEY
            """
        ),
        encoding="utf-8",
    )
    monkeypatch.setenv("FWLLM_SERVER__PORT", "9090")
    monkeypatch.setenv("FWLLM_REDIS_URL", "redis://cache:6379/1")
    cfg = load_config(cfg_file)
    assert cfg.server.port == 9090
    assert cfg.redis_url == "redis://cache:6379/1"


def test_api_key_resolved_from_env(tmp_path, monkeypatch):
    monkeypatch.setenv("OPENROUTER_API_KEY", "sk-test-123")
    cfg_file = tmp_path / "fwllm.yaml"
    cfg_file.write_text(
        dedent(
            """
            providers:
              openrouter:
                base_url: https://openrouter.ai/api/v1
                api_key_env: OPENROUTER_API_KEY
            """
        ),
        encoding="utf-8",
    )
    cfg = load_config(cfg_file)
    provider = cfg.providers["openrouter"]
    assert provider.api_key == "sk-test-123"


def test_missing_api_key_env_var_raises(tmp_path, monkeypatch):
    monkeypatch.delenv("MISSING_KEY", raising=False)
    cfg_file = tmp_path / "fwllm.yaml"
    cfg_file.write_text(
        "providers:\n  x:\n    base_url: https://x.example/v1\n    api_key_env: MISSING_KEY\n",
        encoding="utf-8",
    )
    with pytest.raises(ConfigError, match="MISSING_KEY"):
        load_config(cfg_file)
