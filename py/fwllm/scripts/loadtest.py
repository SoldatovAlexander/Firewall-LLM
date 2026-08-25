"""Simple async load test for the fwllm gateway.

Usage:
    python scripts/loadtest.py --url http://host:8080 --key TOKEN \
        [--rps 20] [--duration 30]
"""

import argparse
import asyncio
import time

import httpx


async def worker(
    client: httpx.AsyncClient,
    url: str,
    key: str,
    stop_at: float,
    stats: dict,
    lock: asyncio.Lock,
) -> None:
    while time.monotonic() < stop_at:
        start = time.perf_counter()
        code = 0
        try:
            response = await client.post(
                url,
                json={
                    "model": "gpt-4o",
                    "messages": [{"role": "user", "content": "ping"}],
                },
                headers={"Authorization": f"Bearer {key}"},
                timeout=30.0,
            )
            code = response.status_code
        except httpx.HTTPError:
            code = -1
        elapsed = time.perf_counter() - start
        async with lock:
            stats["total"] += 1
            stats["latencies"].append(elapsed)
            bucket = "2xx" if 200 <= code < 300 else str(code)
            stats["codes"][bucket] = stats["codes"].get(bucket, 0) + 1
        if code == 429 or code == -1:
            await asyncio.sleep(1.0)


async def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", required=True)
    parser.add_argument("--key", required=True)
    parser.add_argument("--rps", type=int, default=20)
    parser.add_argument("--duration", type=int, default=30)
    args = parser.parse_args()

    concurrency = max(args.rps // 5, 1)
    lock = asyncio.Lock()
    stats: dict = {"total": 0, "latencies": [], "codes": {}}
    stop_at = time.monotonic() + args.duration

    async with httpx.AsyncClient() as client:
        workers = [
            asyncio.create_task(
                worker(client, args.url, args.key, stop_at, stats, lock)
            )
            for _ in range(concurrency)
        ]
        await asyncio.gather(*workers)

    latencies = sorted(stats["latencies"])
    if latencies:
        p50 = latencies[len(latencies) // 2]
        p95 = latencies[int(len(latencies) * 0.95)]
        print(f"requests : {stats['total']}")
        print(f"rps      : {stats['total'] / args.duration:.1f}")
        print(f"p50      : {p50 * 1000:.0f} ms")
        print(f"p95      : {p95 * 1000:.0f} ms")
        print("codes    :", stats["codes"])


if __name__ == "__main__":
    asyncio.run(main())
