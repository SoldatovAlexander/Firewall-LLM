"""Configuration loading: YAML file + FWLLM_* env overrides + api_key resolution."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Literal

import pydantic
import yaml
from pydantic import BaseModel, Field, ValidationError


class ConfigError(Exception):
    """Raised when configuration is missing, unparsable or invalid."""


ProviderType = Literal["openai_compat", "openrouter", "ollama"]


class ServerConfig(BaseModel):
    host: str = "127.0.0.1"
    port: int = 8080
    request_timeout_seconds: float = 120.0


class ProviderConfig(BaseModel):
    type: ProviderType = "openai_compat"
    base_url: str
    api_key_env: str | None = None
    api_key: str | None = Field(default=None, exclude=True)
    models: list[str] = Field(default_factory=list)


class Quotas(BaseModel):
    client_tokens_per_day: int | None = None
    client_requests_per_day: int | None = None
    provider_tokens_per_day: int | None = None


class DLPConfig(BaseModel):
    mode: Literal["block", "mask", "log", "off"] = "mask"
    restore_policy: Literal["restore", "mask"] = "mask"
    profile: str = "ru_152"


class InjectionConfig(BaseModel):
    mode: Literal["block", "log", "off"] = "block"
    block_severity_gte: Literal["low", "medium", "high", "critical"] = "high"


class InspectorsConfig(BaseModel):
    dlp: DLPConfig = Field(default_factory=DLPConfig)
    injection: InjectionConfig = Field(default_factory=InjectionConfig)


class EgressConfig(BaseModel):
    """Outbound policy. MVP: direct or one global proxy (pools are enterprise)."""

    mode: Literal["direct", "single_proxy"] = "direct"
    proxy_url: str | None = None

    @pydantic.model_validator(mode="after")
    def _proxy_required_in_single_mode(self) -> EgressConfig:
        if self.mode == "single_proxy" and not self.proxy_url:
            raise ValueError("egress.single_proxy requires proxy_url")
        return self


class Threshold(BaseModel):
    gt: float | None = None
    gte: float | None = None
    lt: float | None = None
    lte: float | None = None

    def matches(self, value: float) -> bool:
        if self.gt is not None and not value > self.gt:
            return False
        if self.gte is not None and not value >= self.gte:
            return False
        if self.lt is not None and not value < self.lt:
            return False
        if self.lte is not None and not value <= self.lte:
            return False
        return True


class RuleCondition(BaseModel):
    provider: str | None = None
    provider_tokens_today: Threshold | None = None


class RuleAction(BaseModel):
    next_in_chain: bool = False
    switch_to: str | None = None


class RoutingRule(BaseModel):
    name: str
    when: RuleCondition
    action: RuleAction


class AttackFailoverConfig(BaseModel):
    enabled: bool = False
    count: int = 3
    window_seconds: int = 300
    min_severity: Literal["low", "medium", "high", "critical"] = "high"
    switch_to: str | None = None
    block_source: bool = True
    block_ttl_seconds: int = 600
    cooldown_seconds: int = 300


class RoutingConfig(BaseModel):
    default_chain: list[str] = Field(default_factory=list)
    model_mapping: dict[str, dict[str, str]] = Field(default_factory=dict)
    rules: list[RoutingRule] = Field(default_factory=list)
    attack_failover: AttackFailoverConfig = Field(default_factory=AttackFailoverConfig)


class AuditConfig(BaseModel):
    enabled: bool = True
    db_path: str = "audit.db"
    dlp_redact: bool = True


class Config(BaseModel):
    server: ServerConfig = Field(default_factory=ServerConfig)
    redis_url: str = "redis://localhost:6379/0"
    providers: dict[str, ProviderConfig]
    # client API token -> label
    clients: dict[str, str] = Field(default_factory=dict)
    quotas: Quotas = Field(default_factory=Quotas)
    inspectors: InspectorsConfig = Field(default_factory=InspectorsConfig)
    egress: EgressConfig = Field(default_factory=EgressConfig)
    routing: RoutingConfig = Field(default_factory=RoutingConfig)
    audit: AuditConfig = Field(default_factory=AuditConfig)

    model_config = {"frozen": True}


def _resolve_api_keys(cfg: Config) -> Config:
    data = cfg.model_dump()
    for name, p in cfg.providers.items():
        env_var = p.api_key_env
        if env_var:
            value = os.environ.get(env_var)
            if not value:
                raise ConfigError(
                    f"config error: environment variable '{env_var}' "
                    f"(api_key_env of provider '{name}') is not set"
                )
            data["providers"][name]["api_key"] = value
    return Config.model_validate(data)


def load_config(path: Path | str) -> Config:
    path = Path(path)
    if not path.is_file():
        raise ConfigError(f"config file not found: {path}")
    try:
        raw = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    except yaml.YAMLError as exc:
        raise ConfigError(f"failed to parse config: {exc}") from exc
    if not isinstance(raw, dict):
        raise ConfigError("failed to parse config: top level must be a mapping")

    env_port = os.environ.get("FWLLM_SERVER__PORT")
    if env_port:
        raw.setdefault("server", {})["port"] = int(env_port)
    env_redis = os.environ.get("FWLLM_REDIS_URL")
    if env_redis:
        raw["redis_url"] = env_redis

    try:
        cfg = Config.model_validate(raw)
    except ValidationError as exc:
        messages = "; ".join(
            f"{'.'.join(str(p) for p in e['loc'])}: {e['msg']}" for e in exc.errors()
        )
        raise ConfigError(f"invalid config: {messages}") from exc
    return _resolve_api_keys(cfg)
