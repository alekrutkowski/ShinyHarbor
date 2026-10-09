"""Inherited frame headers and opt-in tracking markup without contacting Google."""
import argparse
import asyncio
import json
from test_support import TemporaryDirectory
from pathlib import Path
from aiohttp import ClientSession
from gateway import free_port, wait_ready

ROOT = Path(__file__).resolve().parents[1]


async def main(binary):
    results = ROOT / 'test-results'
    results.mkdir(exist_ok=True)
    checks = []
    with TemporaryDirectory(prefix='harbor-pages-') as temp:
        root = Path(temp)
        port = free_port()
        config = root / 'server.conf'
        source = f'''google_analytics_id G-TEST123;
frame_options allow-from https://embed.example;
server {{ listen {port} 127.0.0.1;
location /tracked {{ app_dir "{ROOT / 'examples/hello'}"; }}
location /plain {{ app_dir "{ROOT / 'examples/hello'}"; google_analytics_id ""; frame_options allow; }}
}}'''
        config.write_text(source)
        with (results / 'page-settings.log').open('wb') as log:
            process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=log, stderr=log)
            try:
                await wait_ready(3839, process)
                async with ClientSession() as client:
                    async with client.get(f'http://127.0.0.1:{port}/tracked/') as response:
                        body = await response.text()
                        assert response.status == 200
                        assert response.headers['X-Frame-Options'] == 'ALLOW-FROM https://embed.example'
                        assert 'googletagmanager.com/gtag/js?id=G-TEST123' in body
                        assert 'gtag("config","G-TEST123")' in body
                        assert body.index('gtag("config"') < body.index('</head>')
                        checks += ['inherited-allow-from-header', 'inherited-Google-Analytics-markup']
                    async with client.get(f'http://127.0.0.1:{port}/plain/') as response:
                        assert response.status == 200
                        assert 'X-Frame-Options' not in response.headers
                        assert 'googletagmanager' not in await response.text()
                        checks.append('location-overrides-disable-tracking-and-framing-header')
                config.write_text(source.replace('G-TEST123', '<script>'))
                command = await asyncio.create_subprocess_exec(str(binary.resolve()), 'check', '--config', str(config), stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
                _, error = await command.communicate()
                assert command.returncode != 0 and b'Invalid Google Analytics tag ID' in error
                checks.append('script-injection-rejected-in-configuration')
            finally:
                process.terminate()
                await asyncio.wait_for(process.wait(), 20)
    report = {'checks': checks, 'external_analytics_contacted': False}
    (results / 'page-settings.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    asyncio.run(main(parser.parse_args().binary))
