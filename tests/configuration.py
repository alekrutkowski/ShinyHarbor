"""Real HTTP checks for legacy listeners and virtual host isolation."""
import argparse
import asyncio
import json
import tempfile
from pathlib import Path
from aiohttp import ClientSession
from gateway import free_port, wait_ready

async def main(binary):
    with tempfile.TemporaryDirectory(prefix="harbor-config-") as temp:
        root = Path(temp)
        ports = {free_port() for _ in range(3)}
        while len(ports) < 3 or 3839 in ports:
            ports = {free_port() for _ in range(3)}
        first, second, _ = sorted(ports)
        sites = []
        for value in ["first", "second", "third"]:
            site = root / value
            site.mkdir()
            (site / "index.html").write_text(value, encoding="utf-8")
            sites.append(site.as_posix())
        config = root / "shiny-server.conf"
        config.write_text(f"""
server {{ listen {first} 127.0.0.1; server_name first.example; location / {{ site_dir "{sites[0]}"; }} }}
server {{ listen {first} 127.0.0.1; server_name second.example; location / {{ site_dir "{sites[1]}"; }} }}
server {{ listen {second} 127.0.0.1; location / {{ site_dir "{sites[2]}"; }} }}
""", encoding="utf-8")
        with (root / "server.log").open("wb") as log:
            proc = await asyncio.create_subprocess_exec(str(binary.resolve()), "serve", "--config", str(config), stdout=log, stderr=log)
            try:
                await wait_ready(3839, proc)
                async with ClientSession() as client:
                    for port, host, expected in [(first,"first.example","first"), (first,"SECOND.EXAMPLE:1234","second"), (second,"anything.example","third")]:
                        async with client.get(f"http://127.0.0.1:{port}/", headers={"Host":host}) as response:
                            assert response.status == 200, await response.text()
                            assert await response.text() == expected
                    async with client.get(f"http://127.0.0.1:{first}/", headers={"Host":"unknown.example"}) as response:
                        assert response.status == 404
            finally:
                proc.terminate()
                await asyncio.wait_for(proc.wait(), 10)
    return {"suite":"legacy-virtual-hosts", "passed":True, "checks":["multiple-listeners", "host-routing", "host-isolation", "case-insensitive-host", "host-port-ignored", "unnamed-host-fallback"]}

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    report = asyncio.run(main(parser.parse_args().binary))
    Path("test-results").mkdir(exist_ok=True)
    Path("test-results/configuration.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, indent=2))
