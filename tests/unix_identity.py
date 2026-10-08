"""Run as root on Unix to verify child credential isolation using an existing user."""
import argparse
import asyncio
import grp
import json
import os
import pwd
import tempfile
from pathlib import Path
from aiohttp import ClientSession
from gateway import free_port, wait_ready

async def main(binary, account, sockets=False):
    assert os.geteuid() == 0, "This test needs root to exercise dropping privileges"
    user = pwd.getpwnam(account)
    assert user.pw_uid != 0, "Choose an unprivileged test account"
    with tempfile.TemporaryDirectory(prefix="harbor-unix-identity-") as temp:
        root = Path(temp)
        root.chmod(0o755)
        app = root / "app"
        app.mkdir(mode=0o755)
        (app / "app.R").write_text('library(shiny)\nshinyApp(fluidPage(tags$pre(id="account", Sys.info()[["effective_user"]]), tags$pre(id="groups", system("id -G", intern=TRUE))), function(input, output, session) {})\n')
        config = root / "harbor.toml"
        public, admin = free_port(), free_port()
        config.write_text(f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\nshutdown_grace_seconds=1\n"
                          + ("socket_dir='sockets'\n" if sockets else "") +
                          f"[[apps]]\nname='identity'\nmount='/identity'\npath='{app}'\n"
                          f"run_as=['harbor-no-such-account', '{account}']\n")
        with (root / "server.log").open("wb") as log:
            proc = await asyncio.create_subprocess_exec(str(binary.resolve()), "serve", "--config", str(config), stdout=log, stderr=log)
            try:
                await wait_ready(admin, proc)
                async with ClientSession() as client:
                    async with client.get(f"http://127.0.0.1:{public}/identity/") as response:
                        body = await response.text()
                        if response.status != 200:
                            print((root / "server.log").read_text())
                            for path in (root / ".harbor/logs").glob("*.log"):
                                print(path.name, path.read_text())
                        assert response.status == 200, body
                        assert f'id="account">{account}</pre>' in body, body
                        expected = " ".join(str(g) for g in sorted(os.getgrouplist(account, user.pw_gid)))
                        import re
                        actual = re.search(r'id="groups">([^<]+)</pre>', body).group(1)
                        assert sorted(actual.split()) == sorted(expected.split()), (actual, expected)
                assert (root / ".harbor").stat().st_mode & 0o077 == 0
                assert (root / ".harbor/admin.token").stat().st_uid == 0
                assert (root / ".harbor.users" / str(user.pw_uid)).stat().st_uid == user.pw_uid
            finally:
                proc.terminate()
                await asyncio.wait_for(proc.wait(), 15)
    return {"suite":"unix-identity", "passed":True, "checks":["named-user-fallback", "worker-uid", "supplementary-groups", "private-gateway-state", "user-owned-bookmarks"]}

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--account", default="nobody")
    parser.add_argument("--sockets", action="store_true")
    args = parser.parse_args()
    report = asyncio.run(main(args.binary, args.account, args.sockets))
    report['unix_domain_sockets'] = args.sockets
    Path("test-results").mkdir(exist_ok=True)
    Path("test-results/unix-identity.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
