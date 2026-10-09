"""Mixed HTTP/HTTPS listeners, independent certificates and transport metadata."""
import argparse
import asyncio
import contextlib
import ipaddress
import json
import ssl
from test_support import TemporaryDirectory
from datetime import datetime, timedelta, timezone
from pathlib import Path

from aiohttp import ClientSession, ClientTimeout, web
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID
from gateway import free_port, wait_ready


def make_certificate(root, name):
    key = ec.generate_private_key(ec.SECP256R1())
    subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, name)])
    now = datetime.now(timezone.utc)
    cert = (x509.CertificateBuilder().subject_name(subject).issuer_name(subject)
            .public_key(key.public_key()).serial_number(x509.random_serial_number())
            .not_valid_before(now - timedelta(minutes=1)).not_valid_after(now + timedelta(days=1))
            .add_extension(x509.SubjectAlternativeName([x509.DNSName('localhost'),
                x509.IPAddress(ipaddress.ip_address('127.0.0.1'))]), critical=False)
            .sign(key, hashes.SHA256()))
    pem = cert.public_bytes(serialization.Encoding.PEM)
    (root / f'{name}.pem').write_bytes(pem)
    (root / f'{name}.key').write_bytes(key.private_bytes(serialization.Encoding.PEM,
        serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
    return pem.decode()


async def certificate(port, trust):
    _, writer = await asyncio.wait_for(asyncio.open_connection('127.0.0.1', port,
        ssl=trust, server_hostname='localhost'), 3)
    try:
        return writer.get_extra_info('ssl_object').getpeercert()['serialNumber']
    finally:
        writer.close()
        with contextlib.suppress(ConnectionError):
            await writer.wait_closed()


async def stop(process):
    if process and process.returncode is None:
        process.terminate()
        await asyncio.wait_for(process.wait(), 15)


async def main(binary):
    checks = []
    results = Path('test-results')
    results.mkdir(exist_ok=True)
    with TemporaryDirectory(prefix='harbor-listener-tls-') as temp:
        root = Path(temp)
        (root / 'index.html').write_text('mixed listeners')
        first_pem = make_certificate(root, 'first')
        second_pem = make_certificate(root, 'second')
        trust = ssl.create_default_context(cadata=first_pem + second_pem)
        ports = set()
        while len(ports) < 4:
            port = free_port()
            if port != 3839:
                ports.add(port)
        first, second, plain, admin = sorted(ports)
        legacy = root / 'shiny-server.conf'
        legacy.write_text('\n'.join([
            f'server {{ listen {first} 127.0.0.1; ssl first.key first.pem; location / {{ site_dir .; }} }}',
            f'server {{ listen {second} 127.0.0.1; ssl second.key second.pem; location / {{ site_dir .; }} }}',
            f'server {{ listen {plain} 127.0.0.1; location / {{ site_dir .; }} }}',
        ]), encoding='utf-8')
        process = None
        with (results / 'listener-tls.log').open('wb') as log:
            try:
                process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config',
                    str(legacy), stdout=log, stderr=log)
                await wait_ready(3839, process)
                first_serial = await certificate(first, trust)
                second_serial = await certificate(second, trust)
                assert first_serial != second_serial
                checks.append('legacy-listeners-present-independent-trusted-certificates')
                async with ClientSession(timeout=ClientTimeout(total=5)) as client:
                    for port, scheme in [(first, 'https'), (second, 'https'), (plain, 'http')]:
                        async with client.get(f'{scheme}://127.0.0.1:{port}/', ssl=trust) as response:
                            assert response.status == 200 and await response.text() == 'mixed listeners'
                    checks.append('legacy-HTTP-and-HTTPS-coexist')
                    for name in ['first.key', 'second.key']:
                        async with client.get(f'http://127.0.0.1:{plain}/{name}') as response:
                            assert response.status == 404
                    checks.append('every-listener-private-key-blocked-on-static-routes')
                replacement = make_certificate(root, 'first')
                new_trust = ssl.create_default_context(cadata=replacement + second_pem)
                async with asyncio.timeout(15):
                    while True:
                        try:
                            serial = await certificate(first, new_trust)
                            if serial != first_serial:
                                break
                        except OSError:
                            pass
                        await asyncio.sleep(0.25)
                assert await certificate(second, new_trust) == second_serial
                checks.append('certificate-reload-is-isolated-to-own-listener')
            finally:
                await stop(process)

            async def backend(request):
                return web.Response(text=request.headers.get('X-Forwarded-Proto', 'missing'))
            app = web.Application()
            app.router.add_get('/', backend)
            runner = web.AppRunner(app)
            await runner.setup()
            site = web.TCPSite(runner, '127.0.0.1', 0)
            await site.start()
            backend_port = site._server.sockets[0].getsockname()[1]
            config = root / 'harbor.toml'
            content = (f"listen='127.0.0.1:{first}'\nadmin_listen='127.0.0.1:{admin}'\n"
                       "state_dir='toml-state'\nshutdown_grace_seconds=1\n"
                       "[tls]\nmode='pem'\ncert='first.pem'\nkey='first.key'\n"
                       f"[[servers]]\nlisten='127.0.0.1:{first}'\n"
                       f"[[servers]]\nlisten='127.0.0.1:{plain}'\ntls={{mode='off'}}\n")
            for number in range(2):
                content += (f"[[upstreams]]\nserver={number}\nname='route{number}'\nmount='/'\n"
                            f"backends=['http://127.0.0.1:{backend_port}']\n")
            config.write_text(content, encoding='utf-8')
            try:
                process = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config',
                    str(config), stdout=log, stderr=log)
                await wait_ready(admin, process)
                async with ClientSession(timeout=ClientTimeout(total=5)) as client:
                    for port, scheme in [(first, 'https'), (plain, 'http')]:
                        async with client.get(f'{scheme}://127.0.0.1:{port}/', ssl=new_trust) as response:
                            assert response.status == 200 and await response.text() == scheme
                            cookies = response.headers.getall('Set-Cookie')
                            assert len(cookies) == 1
                            assert ('; Secure' in cookies[0]) == (scheme == 'https'), cookies
                checks += ['TOML-inheritance-and-plaintext-override', 'forwarded-scheme-matches-listener',
                           'affinity-cookie-security-matches-listener']
            finally:
                await stop(process)
                await runner.cleanup()
            conflict = root / 'conflict.toml'
            conflict.write_text(content.replace(f"listen='127.0.0.1:{plain}'", f"listen='127.0.0.1:{first}'"), encoding='utf-8')
            check = await asyncio.create_subprocess_exec(str(binary.resolve()), 'check', '--config', str(conflict),
                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
            out, err = await asyncio.wait_for(check.communicate(), 10)
            assert check.returncode != 0 and b'must use the same TLS settings' in out + err, (out, err)
            checks.append('conflicting-TLS-on-shared-address-rejected')
    report = {'suite': 'listener-TLS', 'passed': True, 'checks': checks,
              'certificate_verification_disabled': False}
    (results / 'listener-tls.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    print(json.dumps(asyncio.run(main(parser.parse_args().binary)), indent=2))
