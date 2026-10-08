#!/usr/bin/env python3
"""Executable gateway integration checks. Requires a compiled server, not R."""
from __future__ import annotations
import argparse
import asyncio
import base64
import contextlib
import hashlib
import json
import os
import socket
import tempfile
from pathlib import Path
from aiohttp import ClientSession, ClientTimeout, CookieJar, WSMsgType, web
from yarl import URL


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


async def wait_ready(port: int, process: asyncio.subprocess.Process) -> None:
    async with ClientSession(timeout=ClientTimeout(total=1)) as client:
        for _ in range(100):
            if process.returncode is not None:
                raise RuntimeError(f"Server exited during startup ({process.returncode})")
            try:
                async with client.get(f"http://127.0.0.1:{port}/healthz") as response:
                    if response.status == 200:
                        return
            except (OSError, asyncio.TimeoutError):
                pass
            await asyncio.sleep(0.1)
    raise TimeoutError("Admin listener did not become ready")


async def backend(request: web.Request) -> web.StreamResponse:
    ws = web.WebSocketResponse(autoping=True)
    if ws.can_prepare(request).ok:
        await ws.prepare(request)
        async for msg in ws:
            if msg.type == WSMsgType.TEXT:
                await ws.send_str(msg.data)
            elif msg.type == WSMsgType.BINARY:
                await ws.send_bytes(msg.data)
        return ws
    if request.path == "/stream":
        response = web.StreamResponse(headers={"Content-Type": "application/octet-stream"})
        await response.prepare(request)
        for number in range(8):
            await response.write(bytes([number]) * 65536)
            await asyncio.sleep(0.02)
        await response.write_eof()
        return response
    if request.path == "/failure":
        return web.Response(status=503, text="backend unavailable")
    body = await request.read()
    response = web.json_response({"backend": request.app["identity"], "raw_path": request.raw_path,
        "method": request.method, "length": len(body), "sha256": hashlib.sha256(body).hexdigest(),
        "xff": request.headers.get("X-Forwarded-For"), "identity": request.headers.get("X-Harbor-User"),
        "host": request.headers.get("Host"), "private": request.headers.get("X-Private")})
    response.headers.add("Set-Cookie", "backend-one=1; Path=/")
    response.headers.add("Set-Cookie", "backend-two=2; Path=/")
    return response


async def check(binary: Path) -> dict:
    binary = binary.resolve()
    if not binary.is_file():
        raise FileNotFoundError(binary)
    runners = []
    for identity in ["a", "b"]:
        app = web.Application(client_max_size=16 * 1024 * 1024)
        app["identity"] = identity
        app.router.add_route("*", "/{tail:.*}", backend)
        runner = web.AppRunner(app)
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", 0)
        await site.start()
        runners.append((runner, site._server.sockets[0].getsockname()[1]))
    checks = []
    try:
        with tempfile.TemporaryDirectory(prefix="harbor-integration-") as temp:
            root = Path(temp)
            public, admin = free_port(), free_port()
            while admin == public:
                admin = free_port()
            conf = root / "harbor.toml"
            backends = [f"http://127.0.0.1:{port}" for _, port in runners]
            conf.write_text(f"listen = '127.0.0.1:{public}'\nadmin_listen = '127.0.0.1:{admin}'\n"
                            "shutdown_grace_seconds = 1\n[[upstreams]]\nname = 'test'\nmount = '/'\n"
                            f"backends = {json.dumps(backends)}\n", encoding="utf-8")
            log_path = root / "server.log"
            with log_path.open("wb") as log:
                process = await asyncio.create_subprocess_exec(str(binary), "serve", "--config", str(conf), stdout=log, stderr=log)
                try:
                    await wait_ready(admin, process)
                    token = (root / ".harbor/admin.token").read_text(encoding="utf-8").strip()
                    base = f"http://127.0.0.1:{public}"
                    # Inspect unencoded wire bodies here; negotiated gzip/deflate
                    # are tested independently in http_compression.py.
                    async with ClientSession(cookie_jar=CookieJar(unsafe=True), timeout=ClientTimeout(total=15), auto_decompress=False, headers={"Accept-Encoding": "identity"}) as client:
                        async with client.get(URL(base + "/a%20b?x=1&x=2&q=%2F", encoded=True), headers={"X-Forwarded-For": "8.8.8.8", "X-Harbor-User": "spoof", "Connection": "x-private", "X-Private": "secret"}) as response:
                            assert response.status == 200
                            row = await response.json()
                            assert row["raw_path"] == "/a%20b?x=1&x=2&q=%2F", row
                            assert row["xff"] == "127.0.0.1" and row["identity"] is None and row["private"] is None, row
                            assert len(response.headers.getall("Set-Cookie")) == 3
                            first_backend = row["backend"]
                        checks += ["raw-path-query", "forwarded-ip-trust", "identity-spoofing", "connection-headers", "multiple-set-cookie"]
                        for _ in range(6):
                            async with client.get(base + "/affinity") as response:
                                assert (await response.json())["backend"] == first_backend
                        checks.append("signed-session-affinity")
                        payload = os.urandom(2 * 1024 * 1024)
                        async def upload():
                            for start in range(0, len(payload), 16384):
                                yield payload[start:start + 16384]
                                await asyncio.sleep(0)
                        async with client.post(base + "/upload", data=upload()) as response:
                            row = await response.json()
                            assert row["length"] == len(payload) and row["sha256"] == hashlib.sha256(payload).hexdigest()
                        checks.append("chunked-upload-integrity")
                        async with client.get(base + "/stream") as response:
                            result = await response.read()
                            assert result == b"".join(bytes([n]) * 65536 for n in range(8))
                        checks.append("streamed-response-integrity")
                        async with client.ws_connect(base + "/socket") as ws:
                            await ws.send_str("Zażółć 日本語")
                            assert (await ws.receive()).data == "Zażółć 日本語"
                            data = bytes(range(256)) * 32
                            await ws.send_bytes(data)
                            assert (await ws.receive()).data == data
                            await ws.ping(b"health")
                        checks += ["websocket-text", "websocket-binary"]
                        async with client.get(base + "/failure") as response:
                            assert response.status == 503
                        checks.append("backend-status-preserved")
                        admin_url = f"http://127.0.0.1:{admin}"
                        async with client.get(admin_url + "/api/stats") as response:
                            assert response.status == 401
                        async with client.get(admin_url + "/api/stats", headers={"Authorization": "Bearer " + token}) as response:
                            assert response.status == 200
                            stats = await response.json()
                            assert stats["apps"][0]["requests"] >= 10 and stats["apps"][0]["websockets"] == 1
                        checks += ["admin-token-required", "real-analytics"]
                        async with client.get(admin_url + "/") as response:
                            assert "frame-ancestors 'none'" in response.headers["Content-Security-Policy"]
                        checks.append("dashboard-security-headers")
                except BaseException:
                    log.flush()
                    print(log_path.read_text(encoding="utf-8", errors="replace"))
                    raise
                finally:
                    if process.returncode is None:
                        process.terminate()
                        try:
                            await asyncio.wait_for(process.wait(), 10)
                        except asyncio.TimeoutError:
                            process.kill()
                            await process.wait()
    finally:
        for runner, _ in runners:
            await runner.cleanup()
    return {"suite": "gateway-integration", "checks": checks, "count": len(checks), "passed": True}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    result = asyncio.run(check(parser.parse_args().binary))
    Path("test-results").mkdir(exist_ok=True)
    Path("test-results/gateway.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))
