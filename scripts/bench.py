#!/usr/bin/env python3
"""Bench Python vs Rust gateway (healthz + chat completions)."""
import argparse, asyncio, time, statistics
import httpx

async def bench(url: str, key: str, path: str, body=None, rps=50, duration=10):
    async with httpx.AsyncClient(timeout=30) as client:
        end = time.monotonic() + duration
        latencies = []
        codes = {}
        total = 0
        sem = asyncio.Semaphore(rps)
        async def one():
            nonlocal total
            async with sem:
                start = time.perf_counter()
                try:
                    if body:
                        r = await client.post(url+path, json=body, headers={"Authorization": f"Bearer {key}"})
                    else:
                        r = await client.get(url+path)
                    code = r.status_code
                except Exception:
                    code = -1
                lat = time.perf_counter() - start
                latencies.append(lat)
                codes[code] = codes.get(code, 0) + 1
                total += 1
        tasks = []
        # fire at rps
        while time.monotonic() < end:
            tasks.append(asyncio.create_task(one()))
            await asyncio.sleep(1/rps)
        await asyncio.gather(*tasks)
        latencies.sort()
        p50 = latencies[len(latencies)//2]*1000 if latencies else 0
        p95 = latencies[int(len(latencies)*0.95)]*1000 if latencies else 0
        return {"total": total, "rps": total/duration, "p50": p50, "p95": p95, "codes": codes}

async def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--py-url", default="http://192.168.88.101:8080")
    ap.add_argument("--rust-url", default="http://192.168.88.101:8081")
    ap.add_argument("--key", default="test-client-key")
    ap.add_argument("--rps", type=int, default=50)
    ap.add_argument("--duration", type=int, default=15)
    args = ap.parse_args()
    body = {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}
    print(f"Bench {args.rps} rps x {args.duration}s")
    for name, url in [("Python", args.py_url), ("Rust", args.rust_url)]:
        print(f"\n== {name} {url} ==")
        for path, b in [("/healthz", None), ("/v1/chat/completions", body)]:
            res = await bench(url, args.key, path, b, args.rps, args.duration)
            print(f"{path:25s} total={res['total']:4d} rps={res['rps']:5.1f} p50={res['p50']:5.0f}ms p95={res['p95']:5.0f}ms codes={res['codes']}")

if __name__ == "__main__":
    asyncio.run(main())
