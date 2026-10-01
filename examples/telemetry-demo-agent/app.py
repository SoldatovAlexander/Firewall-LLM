import os
import time

import requests
from opentelemetry import propagate, trace
from opentelemetry.exporter.otlp.proto.grpc.trace_exporter import OTLPSpanExporter
from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor

FWLLM_URL = os.getenv(
    "FWLLM_URL", "http://localhost:8081/v1/chat/completions"
)
FWLLM_TOKEN = os.getenv("FWLLM_TOKEN", "prototype-token")
OTLP_ENDPOINT = os.getenv(
    "OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4317"
)

provider = TracerProvider(
    resource=Resource.create({"service.name": "fwllm-demo-agent"})
)
provider.add_span_processor(
    BatchSpanProcessor(
        OTLPSpanExporter(endpoint=OTLP_ENDPOINT, insecure=True)
    )
)
trace.set_tracer_provider(provider)
tracer = trace.get_tracer("fwllm-demo-agent")


def wait_for_gateway() -> None:
    health_url = FWLLM_URL.rsplit("/v1/chat/completions", 1)[0] + "/healthz"
    last_error = None
    for _ in range(60):
        try:
            response = requests.get(health_url, timeout=1)
            if response.ok:
                return
        except requests.RequestException as exc:
            last_error = exc
        time.sleep(1)
    raise RuntimeError(f"gateway did not become ready: {last_error}")


def call_fwllm(user_text: str) -> dict:
    headers = {
        "Authorization": f"Bearer {FWLLM_TOKEN}",
        "Content-Type": "application/json",
    }
    propagate.inject(headers)

    response = requests.post(
        FWLLM_URL,
        headers=headers,
        json={
            "model": "prototype-model",
            "messages": [{"role": "user", "content": user_text}],
            "stream": False,
        },
        timeout=15,
    )
    response.raise_for_status()
    return response.json()


def main() -> None:
    wait_for_gateway()

    with tracer.start_as_current_span("invoke_agent") as root_span:
        root_span.set_attribute("fwllm.agent.id", "demo-agent")

        with tracer.start_as_current_span("retrieval") as retrieval_span:
            retrieval_span.set_attribute("retrieval.system", "mock_catalog")
            retrieval_span.set_attribute("retrieval.documents.count", 3)
            time.sleep(0.05)

        first = call_fwllm(
            "Select a suitable catalog item using the retrieved requirements."
        )

        with tracer.start_as_current_span("execute_tool") as tool_span:
            tool_span.set_attribute("tool.name", "product_catalog.search")
            tool_span.set_attribute("tool.type", "local_mock")
            time.sleep(0.05)

        second = call_fwllm(
            "Prepare the final answer using the tool result."
        )

        trace_id = root_span.get_span_context().trace_id
        print(f"TRACE_ID={trace_id:032x}")
        print(f"FIRST_RESPONSE={first.get('choices', [{}])[0].get('message', {}).get('content')}")
        print(f"SECOND_RESPONSE={second.get('choices', [{}])[0].get('message', {}).get('content')}")

    provider.force_flush()


if __name__ == "__main__":
    main()
