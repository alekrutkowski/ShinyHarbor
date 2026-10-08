"""Real R identity, proxy trust, route policies and reconnect authorization."""
from __future__ import annotations
import argparse
import asyncio
import json
import tempfile
import uuid
from pathlib import Path
from aiohttp import ClientSession
from playwright.async_api import async_playwright, expect
from gateway import free_port, wait_ready
from reconnect import Wire

ROOT = Path(__file__).resolve().parents[1]


async def exercise(binary):
    checks = []
    results = ROOT / 'test-results'
    results.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='harbor-auth-') as temp:
        root = Path(temp)
        app = root / 'app'
        app.mkdir()
        (app / 'app.R').write_text('''library(shiny)
shinyApp(fluidPage(textOutput("user"), textOutput("groups")), function(input, output, session) {
  output$user <- renderText(if (is.null(session$user)) "anonymous" else session$user)
  output$groups <- renderText(paste(sort(session$groups), collapse=","))
})
''', encoding='utf-8')
        for mode in ('trusted', 'untrusted', 'legacy'):
            public, admin = free_port(), free_port()
            if mode == 'legacy':
                admin = 3839
                config = root / 'server.conf'
                config.write_text(f'''auth_proxy X-USER X-GROUPS;
run_as :PROCESS_USER:;
server {{ listen {public} 127.0.0.1;
  location /app {{ app_dir "{app.as_posix()}"; required_user *; required_group analysts; }}
  location /public {{ app_dir "{app.as_posix()}"; required_user **; }}
}}
''', encoding='utf-8')
            else:
                config = root / 'harbor.toml'
                config.write_text(f'''listen='127.0.0.1:{public}'
admin_listen='127.0.0.1:{admin}'
shutdown_grace_seconds=1
[auth_proxy]
user_header='x-user'
groups_header='x-groups'
trusted_peers=['{'127.0.0.1/32' if mode == 'trusted' else '127.0.0.2/32'}']
[[apps]]
name='protected'
mount='/app'
path={json.dumps(str(app))}
allowed_users=['*']
allowed_groups=['analysts']
allow_app_override=true
[[apps]]
name='public'
mount='/public'
path={json.dumps(str(app))}
allowed_users=['**']
''', encoding='utf-8')
            with (results / f'auth-{mode}.log').open('wb') as log:
                process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
                try:
                    await wait_ready(admin, process)
                    base = f'http://127.0.0.1:{public}'
                    headers = {'x-user': 'alice@example.org', 'x-groups': 'staff, analysts',
                               'shiny-server-credentials': '{"user":"mallory","groups":["admins"]}',
                               'x-forwarded-for': '127.0.0.1'}
                    async with ClientSession() as client:
                        async with client.get(base + '/app/') as response:
                            assert response.status == 401
                            assert 'WWW-Authenticate' not in response.headers
                        async with client.get(base + '/app/', headers=headers) as response:
                            assert response.status == (401 if mode == 'untrusted' else 200), await response.text()
                        async with client.get(base + '/app/', headers={**headers, 'x-groups': 'staff'}) as response:
                            assert response.status == (401 if mode == 'untrusted' else 403)
                        async with client.get(base + '/app/', headers=[('x-user', 'alice'), ('x-user', 'bob'), ('x-groups', 'analysts')]) as response:
                            assert response.status == 401
                    checks.append(f'{mode}-authorization-and-header-spoofing')
                    if mode == 'untrusted':
                        continue
                    async with async_playwright() as p:
                        browser = await p.chromium.launch(headless=True)
                        try:
                            context = await browser.new_context(extra_http_headers=headers)
                            page = await context.new_page()
                            await page.goto(base + '/app/')
                            await expect(page.locator('#user')).to_have_text('alice@example.org', timeout=30000)
                            await expect(page.locator('#groups')).to_have_text('analysts,staff')
                            anonymous = await browser.new_page()
                            await anonymous.goto(base + '/public/')
                            await expect(anonymous.locator('#user')).to_have_text('anonymous', timeout=30000)
                        finally:
                            await browser.close()
                    checks += [f'{mode}-R-session-identity-and-groups', f'{mode}-anonymous-public-access']
                    if mode == 'trusted':
                        # Open a public transport, then attempt to enter a protected
                        # app through multiplexing: every channel must be checked.
                        async with ClientSession() as client:
                            wire = Wire(client, base + '/public/__sockjs__')
                            await wire.connect()
                            await wire.send('0|o|../app/')
                            message = await wire.next()
                            assert message[:2] == ('0', 'c'), message
                            await wire.ws.close()
                        checks.append('multiplex-cannot-bypass-app-authorization')
                        async with ClientSession(headers=headers) as client:
                            wire = Wire(client, base + '/public/__sockjs__')
                            await wire.connect()
                            await wire.send('0|o|../app/')
                            assert 'config' in (await wire.next())[2]
                            await wire.ws.close()
                            # The parent is public, so its ACL alone cannot catch
                            # a revoked child permission. The entire identity is bound.
                            client.headers['x-groups'] = 'staff'
                            url = f'{wire.base}/o={wire.identifier}/000/{uuid.uuid4().hex}/websocket'
                            async with client.get(url) as response:
                                assert response.status == 503, response.status
                        checks.append('changed-groups-cannot-resume-existing-privileges')
                        override = app / '.shiny_app.conf'
                        try:
                            override.write_text('required_user bob; required_group;', encoding='utf-8')
                            async with ClientSession() as client:
                                async with client.get(base + '/app/', headers=headers) as response:
                                    assert response.status == 403
                                async with client.get(base + '/app/', headers={'x-user': 'bob'}) as response:
                                    assert response.status == 200
                                override.write_text('required_user **; required_group;', encoding='utf-8')
                                async with client.get(base + '/app/') as response:
                                    assert response.status == 200
                            checks.append('app-overrides-change-user-and-group-policy')
                        finally:
                            override.unlink(missing_ok=True)
                finally:
                    if process.returncode is None:
                        process.terminate()
                        await asyncio.wait_for(process.wait(), 20)
    (results / 'auth-proxy.json').write_text(json.dumps({'checks': checks}, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'checks': checks}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    asyncio.run(exercise(parser.parse_args().binary))
