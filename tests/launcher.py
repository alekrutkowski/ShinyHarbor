"""Exercise the embedded R launcher with real HTTP and WebSocket traffic.

Uses only Python's standard library and installed R/Shiny. This checks the R
integration independently; it does not certify the Rust proxy or Shiny Server.
"""
from __future__ import annotations
import base64
import http.client
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import time
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]


class Inputs(HTMLParser):
    def __init__(self):
        super().__init__()
        self.values = {}

    def handle_starttag(self, tag, attrs):
        row = dict(attrs)
        if tag == "input" and row.get("id"):
            self.values[row["id"]] = row.get("value")


def receive_exact(sock, size):
    chunks = bytearray()
    while len(chunks) < size:
        data = sock.recv(size - len(chunks))
        if not data:
            raise EOFError("WebSocket closed")
        chunks.extend(data)
    return bytes(chunks)


def receive_frame(sock):
    first, second = receive_exact(sock, 2)
    length = second & 127
    if length == 126:
        length = struct.unpack("!H", receive_exact(sock, 2))[0]
    elif length == 127:
        length = struct.unpack("!Q", receive_exact(sock, 8))[0]
    assert length <= 16 * 1024 * 1024, "Oversized test frame"
    assert not second & 128, "Server frames must not be masked"
    return first & 15, receive_exact(sock, length)


def send_json(sock, value):
    data = json.dumps(value).encode()
    mask = os.urandom(4)
    prefix = bytes([0x81, 0x80 | len(data)]) if len(data) < 126 else bytes([0x81, 0xFE]) + struct.pack("!H", len(data))
    sock.sendall(prefix + mask + bytes(value ^ mask[i % 4] for i, value in enumerate(data)))


def connect_ws(port, headers):
    sock = socket.create_connection(("127.0.0.1", port), timeout=10)
    sock.settimeout(10)
    key = base64.b64encode(os.urandom(16)).decode()
    request = (f"GET /websocket/ HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n"
               f"Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n")
    request += "".join(f"{name}: {value}\r\n" for name, value in headers.items()) + "\r\n"
    sock.sendall(request.encode())
    response = bytearray()
    while not response.endswith(b"\r\n\r\n"):
        response.extend(receive_exact(sock, 1))
        assert len(response) < 65536
    return sock, bytes(response)


def main():
    results = ROOT / "test-results"
    results.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="harbor-launcher-") as temp:
        state = Path(temp)
        secret = "test-worker-secret-" + os.urandom(24).hex()
        ready = state / "ready"
        env = dict(os.environ, HARBOR_APP=str(ROOT / "examples/session"), HARBOR_READY=str(ready),
                   HARBOR_RUNTIME=str(ROOT / "r/runtime.R"), HARBOR_BOOKMARKS=str(state),
                   HARBOR_SHARED_SECRET=secret, HARBOR_SANITIZE_ERRORS="true")
        with (results / "launcher.log").open("wb") as log:
            proc = subprocess.Popen(["Rscript", "--no-save", "--no-restore", str(ROOT / "r/bootstrap.R")],
                                    env=env, stdout=log, stderr=log, stdin=subprocess.DEVNULL)
            try:
                deadline = time.monotonic() + 45
                while not ready.exists():
                    if proc.poll() is not None:
                        raise RuntimeError((results / "launcher.log").read_text(encoding="utf-8", errors="replace"))
                    if time.monotonic() > deadline:
                        raise TimeoutError("R did not publish readiness")
                    time.sleep(0.05)
                port = urlsplit(ready.read_text(encoding="utf-8").strip()).port
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
                connection.request("GET", "/", headers={"Shiny-Shared-Secret": secret})
                response = connection.getresponse()
                assert response.status == 200
                assert b"Session compatibility fixture" in response.read()
                connection.close()
                for supplied in [{}, {"Shiny-Shared-Secret": "forged", "Shiny-Server-Credentials": '{"user":"attacker"}'}]:
                    sock, response = connect_ws(port, supplied)
                    try:
                        if b" 101 " in response.split(b"\r\n")[0]:
                            try:
                                opcode, payload = receive_frame(sock)
                                assert opcode == 8, (opcode, payload)
                            except (EOFError, ConnectionResetError):
                                pass
                        else:
                            assert b" 403 " in response.split(b"\r\n")[0]
                    finally:
                        sock.close()
                sock, response = connect_ws(port, {"Shiny-Shared-Secret": secret,
                    "Shiny-Server-Credentials": '{"user":"alice","groups":["analysts"]}'})
                try:
                    assert b" 101 " in response.split(b"\r\n")[0]
                    opcode, payload = receive_frame(sock)
                    config = json.loads(payload)
                    assert config["config"]["user"] == "alice", config
                    send_json(sock, {"method": "init", "data": {"n": 7, "save:shiny.action": 0,
                        ".clientdata_output_double_hidden": False, ".clientdata_output_identity_hidden": False,
                        ".clientdata_output_pid_hidden": False, ".clientdata_output_bookmark_url_hidden": False,
                        ".clientdata_url_protocol": "http:", ".clientdata_url_hostname": "127.0.0.1",
                        ".clientdata_url_port": str(port), ".clientdata_url_pathname": "/", ".clientdata_url_search": ""}})
                    values = {}
                    for _ in range(30):
                        opcode, payload = receive_frame(sock)
                        assert opcode == 1, (opcode, payload)
                        values.update(json.loads(payload).get("values", {}))
                        if values.get("double") == "14" and values.get("identity") == "alice":
                            break
                    assert values.get("double") == "14" and values.get("identity") == "alice", values
                    send_json(sock, {"method": "update", "data": {"save:shiny.action": 1}})
                    for _ in range(30):
                        opcode, payload = receive_frame(sock)
                        values.update(json.loads(payload).get("values", {}))
                        if "_state_id_=" in values.get("bookmark_url", ""):
                            break
                    assert "_state_id_=" in values.get("bookmark_url", ""), values
                    assert list(state.rglob("*.rds")), "Bookmark callback did not persist state"
                    bookmark_query = urlsplit(values["bookmark_url"]).query
                finally:
                    sock.close()
                # Actually restart R and load the saved UI state. Merely sending
                # the same input over a new socket would not prove restoration.
                proc.terminate(); proc.wait(timeout=10)
                ready.unlink()
                proc = subprocess.Popen(["Rscript", "--no-save", "--no-restore", str(ROOT / "r/bootstrap.R")],
                                        env=env, stdout=log, stderr=log, stdin=subprocess.DEVNULL)
                deadline = time.monotonic() + 45
                while not ready.exists():
                    if proc.poll() is not None:
                        raise RuntimeError((results / "launcher.log").read_text(encoding="utf-8", errors="replace"))
                    if time.monotonic() > deadline:
                        raise TimeoutError("Restarted R did not publish readiness")
                    time.sleep(0.05)
                port = urlsplit(ready.read_text(encoding="utf-8").strip()).port
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
                connection.request("GET", "/?" + bookmark_query, headers={"Shiny-Shared-Secret": secret})
                response = connection.getresponse()
                assert response.status == 200
                inputs = Inputs()
                inputs.feed(response.read().decode("utf-8"))
                connection.close()
                assert inputs.values.get("n") == "7", inputs.values
            finally:
                proc.terminate()
                try:
                    proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    proc.kill(); proc.wait()
    report = {"suite": "real-r-launcher", "checks": ["http-page", "missing-secret-rejected",
              "forged-secret-rejected", "session-user", "reactivity", "persistent-server-bookmark",
              "bookmark-restored-after-r-restart"],
              "passed": True, "certifies_rust_proxy": False, "certifies_full_compatibility": False}
    (results / "launcher.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
