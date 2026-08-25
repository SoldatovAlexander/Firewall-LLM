"""Contract tests: shared OpenAPI/JSON-Schema contracts stay valid.

These run for both Python and Rust branches - the files under contracts/ are
the single source of truth.
"""

import json
from pathlib import Path

import yaml

CONTRACTS = Path(__file__).resolve().parents[3] / "contracts"


def test_openapi_parses():
    spec = yaml.safe_load((CONTRACTS / "openapi.yaml").read_text(encoding="utf-8"))
    assert spec["info"]["version"]
    assert "/v1/chat/completions" in spec["paths"]


def test_policy_schema_is_valid_json_schema_draft2020():
    from jsonschema import Draft202012Validator

    schema = json.loads((CONTRACTS / "policies.schema.json").read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)


def _validate(policy: dict) -> None:
    from jsonschema import Draft202012Validator

    schema = json.loads((CONTRACTS / "policies.schema.json").read_text(encoding="utf-8"))
    errors = sorted(Draft202012Validator(schema).iter_errors(policy), key=lambda e: e.path)
    assert not errors, [e.message for e in errors]


def test_example_policy_matches_schema():
    policy = yaml.safe_load(
        (CONTRACTS / "policy.example.yaml").read_text(encoding="utf-8")
    )
    _validate(policy)
