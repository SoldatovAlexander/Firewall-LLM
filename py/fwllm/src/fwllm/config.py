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
    # fail-closed: reject requests if metering backend (Redis) is unreachable
    backend_fail_closed: bool = False


class DLPConfig(BaseModel):
    mode: Literal["block", "mask", "log", "off"] = "mask"
    restore_policy: Literal["restore", "mask"] = "mask"
    profile: str = "ru_152"


class MLModelConfig(BaseModel):
    """Local ONNX injection classifier (enterprise module required)."""

    enabled: bool = False
    model_dir: str = ""
    threshold: float = Field(default=0.6, ge=0.0, le=1.0)


class InjectionConfig(BaseModel):
    mode: Literal["block", "log", "off"] = "block"
    block_severity_gte: Literal["low", "medium", "high", "critical"] = "high"
    ml: MLModelConfig = Field(default_factory=MLModelConfig)


class InspectorsConfig(BaseModel):
    dlp: DLPConfig = Field(default_factory=DLPConfig)
    injection: InjectionConfig = Field(default_factory=InjectionConfig)


class PoolConfig(BaseModel):
    proxies: list[str] = Field(min_length=1)
    rotation: Literal["round_robin", "random", "least_used"] = "round_robin"
    requests_per_proxy: int = Field(default=100, ge=1)
    fail_threshold: int = Field(default=3, ge=1)
    cooldown_seconds: int = Field(default=300, ge=1)


class EgressConfig(BaseModel):
    """Outbound policy.

    Open core: direct or one global proxy.
    Enterprise: mode=pools with per-adapter bindings and rotation.
    """

    mode: Literal["direct", "single_proxy", "pools"] = "direct"
    proxy_url: str | None = None
    pools: dict[str, PoolConfig] = Field(default_factory=dict)
    bindings: dict[str, str] = Field(default_factory=dict)

    @pydantic.model_validator(mode="after")
    def _validate(self) -> EgressConfig:
        if self.mode == "single_proxy" and not self.proxy_url:
            raise ValueError("egress.single_proxy requires proxy_url")
        if self.mode == "pools":
            if not self.pools:
                raise ValueError("egress.pools requires at least one pool")
            for adapter, pool in self.bindings.items():
                if pool not in self.pools:
                    raise ValueError(
                        f"binding '{adapter}' references unknown pool '{pool}'"
                    )
        return self


def validate_bindings(cfg: EgressConfig) -> None:
    """Explicit validation helper (raises ValueError on bad bindings)."""
    for adapter, pool in cfg.bindings.items():
        if pool not in cfg.pools:
            raise ValueError(f"binding '{adapter}' references unknown pool '{pool}'")


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
    # memory: state lost on restart; redis: survives restarts (uses redis_url)
    state_store: Literal["memory", "redis"] = "memory"


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
    # admin API tokens (separate from clients) -> label, for /admin/*. If empty, fall back to clients with admin scope.
    admin_clients: dict[str, str] = Field(default_factory=dict)
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

    env_clients = os.environ.get("FWLLM_CLIENT_TOKENS")
    if env_clients:
        clients: dict[str, str] = {}
        for pair in env_clients.split(","):
            pair = pair.strip()
            if not pair:
                continue
            token, _, label = pair.partition(":")
            clients[token] = label or token
        raw["clients"] = clients

    env_admin = os.environ.get("FWLLM_ADMIN_TOKENS")
    if env_admin:
        admin_clients: dict[str, str] = {}
        for pair in env_admin.split(","):
            pair = pair.strip()
            if not pair:
                continue
            token, _, label = pair.partition(":")
            admin_clients[token] = label or token
        raw["admin_clients"] = admin_clients

    try:
        cfg = Config.model_validate(raw)
    except ValidationError as exc:
        messages = "; ".join(
            f"{'.'.join(str(p) for p in e['loc'])}: {e['msg']}" for e in exc.errors()
        )
        raise ConfigError(f"invalid config: {messages}") from exc
    return _resolve_api_keys(cfg)
