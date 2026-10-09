"""TLS integration with a real local Pebble CA (never disables verification).

Provide an official Pebble binary and its test/certs directory. Certificate
lifetimes are shortened in the test CA to exercise actual automatic renewal.
"""
from __future__ import annotations
import argparse
import asyncio
import contextlib
import json
import os
import re
import ssl
import struct
from test_support import TemporaryDirectory
from datetime import datetime, timedelta, timezone
from pathlib import Path
from aiohttp import ClientSession, ClientTimeout
from gateway import free_port, wait_ready
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID

ROOT = Path(__file__).resolve().parents[1]

class TestDNS(asyncio.DatagramProtocol):
    """Loopback-only CA resolver: A is 127.0.0.1, all other types are empty."""
    def connection_made(self, transport):
        self.transport = transport

    def datagram_received(self, data, address):
        reply = self.reply(data)
        if reply:
            self.transport.sendto(reply, address)

    @staticmethod
    def reply(data):
        if len(data) < 17 or data[4:6] != b'\0\1':
            return
        offset = 12
        while offset < len(data) and data[offset]:
            offset += data[offset] + 1
        end = offset + 5
        if end > len(data):
            return
        is_a = data[offset+1:offset+5] == b'\0\1\0\1'
        header = data[:2] + struct.pack('!HHHHH', 0x8180, 1, int(is_a), 0, 0)
        answer = b'\xc0\x0c' + struct.pack('!HHIH', 1, 1, 0, 4) + b'\x7f\0\0\1' if is_a else b''
        return header + data[12:end] + answer

async def dns_stream(reader, writer):
    try:
        while True:
            size = struct.unpack('!H', await reader.readexactly(2))[0]
            reply = TestDNS.reply(await reader.readexactly(size))
            if not reply:
                break
            writer.write(struct.pack('!H', len(reply)) + reply)
            await writer.drain()
    except (asyncio.IncompleteReadError, ConnectionError):
        pass
    finally:
        writer.close()

async def stop(process):
    if process and process.returncode is None:
        process.terminate()
        await asyncio.wait_for(process.wait(), 15)

async def certificate(port, context):
    reader, writer = await asyncio.wait_for(asyncio.open_connection(
        '127.0.0.1', port, ssl=context, server_hostname='localhost'), 3)
    cert = writer.get_extra_info('ssl_object').getpeercert()
    writer.write(b'GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n')
    await writer.drain()
    response = await asyncio.wait_for(reader.read(), 3)
    assert b'200 OK' in response.split(b'\r\n', 1)[0], response[:100]
    writer.close()
    with contextlib.suppress(ConnectionError):
        await writer.wait_closed()
    return cert

async def eventually_certificate(port, context, previous=None, timeout=75):
    async with asyncio.timeout(timeout):
        while True:
            try:
                cert = await certificate(port, context)
                if previous is None or cert['serialNumber'] != previous:
                    return cert
            except (OSError, asyncio.TimeoutError):
                pass
            await asyncio.sleep(0.25)

async def main(binary, pebble, certificates):
    results = ROOT / 'test-results'
    results.mkdir(exist_ok=True)
    checks = []
    with TemporaryDirectory(prefix='harbor-tls-') as temp:
        root = Path(temp)
        (root / 'site').mkdir()
        (root / 'site/index.html').write_text('<h1>TLS test</h1>')
        route = "\n[[sites]]\nname='tls'\nmount='/'\npath='site'\n"
        ca_port, management, public, admin, redirect = [free_port() for _ in range(5)]
        ca_file = certificates / 'pebble.minica.pem'
        cert_file = certificates / 'cert.pem'
        key_file = certificates / 'key.pem'
        if not cert_file.is_file():
            cert_file, key_file = certificates / 'localhost/cert.pem', certificates / 'localhost/key.pem'
        pebble_config = root / 'pebble.json'
        pebble_config.write_text(json.dumps({'pebble': {
            'listenAddress': f'127.0.0.1:{ca_port}', 'managementListenAddress': f'127.0.0.1:{management}',
            'certificate': str(cert_file.resolve()), 'privateKey': str(key_file.resolve()),
            'httpPort': free_port(), 'tlsPort': public, 'ocspResponderURL': '',
            'externalAccountBindingRequired': False,
            'profiles': {'default': {'description': 'short-lived test certificates', 'validityPeriod': 45}}
        }}), encoding='utf-8')
        env = dict(os.environ, PEBBLE_VA_NOSLEEP='1', PEBBLE_WFE_NONCEREJECT='0', PEBBLE_AUTHZREUSE='0')
        env.pop('PEBBLE_VA_ALWAYS_VALID', None)  # The ALPN challenge must really succeed.
        config = root / 'harbor.toml'
        config.write_text(f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\n"
            f"http_redirect_listen='127.0.0.1:{redirect}'\nstate_dir='state'\nshutdown_grace_seconds=1\n"
            "[tls]\nmode='acme'\ndomains=['localhost']\nemail='test@example.org'\n"
            f"directory_url='https://127.0.0.1:{ca_port}/dir'\nca_bundle={json.dumps(str(ca_file.resolve()))}\n" + route,
            encoding='utf-8')
        ca = server = None
        dns, _ = await asyncio.get_running_loop().create_datagram_endpoint(TestDNS, local_addr=('127.0.0.1', 0))
        dns_port = dns.get_extra_info('sockname')[1]
        dns_tcp = await asyncio.start_server(dns_stream, '127.0.0.1', dns_port)
        with (results / 'tls-ca.log').open('wb') as ca_log, (results / 'tls-server.log').open('wb') as server_log:
            try:
                ca = await asyncio.create_subprocess_exec(str(pebble.resolve()), '-config', str(pebble_config),
                    '-strict=false', '-dnsserver', f'127.0.0.1:{dns_port}', env=env, stdout=ca_log, stderr=ca_log)
                ca_trust = ssl.create_default_context(cafile=str(ca_file))
                async with ClientSession(timeout=ClientTimeout(total=3)) as client:
                    async with asyncio.timeout(15):
                        while True:
                            try:
                                async with client.get(f'https://127.0.0.1:{management}/roots/0', ssl=ca_trust) as response:
                                    assert response.status == 200
                                    ca_root = await response.text()
                                    break
                            except OSError:
                                await asyncio.sleep(0.1)
                    trust = ssl.create_default_context(cadata=ca_root)
                    server = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=server_log, stderr=server_log)
                    await wait_ready(admin, server)
                    first = await eventually_certificate(public, trust)
                    checks += ['ACME-TLS-ALPN-01-validation', 'trusted-issued-certificate']
                    for version in (ssl.TLSVersion.TLSv1_2, ssl.TLSVersion.TLSv1_3):
                        context = ssl.create_default_context(cadata=ca_root)
                        context.minimum_version = context.maximum_version = version
                        await certificate(public, context)
                        checks.append(version.name)
                    async with client.get(f'http://127.0.0.1:{redirect}/a%20b?q=x%26y', headers={'Host': 'localhost'}, allow_redirects=False) as response:
                        assert response.status == 308
                        assert response.headers['Location'] == f'https://localhost:{public}/a%20b?q=x%26y'
                    async with client.get(f'http://127.0.0.1:{redirect}/', headers={'Host': 'evil.example'}, allow_redirects=False) as response:
                        assert response.status == 421
                    checks += ['redirect-preserves-path-query', 'redirect-rejects-unconfigured-host']
                    await asyncio.sleep(0.5)
                    await stop(server)
                    server = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(config), stdout=server_log, stderr=server_log)
                    await wait_ready(admin, server)
                    cached = await eventually_certificate(public, trust)
                    assert cached['serialNumber'] == first['serialNumber']
                    checks.append('certificate-cache-survives-restart')
                    renewed = await eventually_certificate(public, trust, first['serialNumber'])
                    assert renewed['notAfter'] != first['notAfter']
                    checks.append('automatic-renewal-without-restart')
                    # Reuse the real issued chain/key to test PEM mode and live replacement.
                    cache = next(p for p in (root / 'state/acme').iterdir() if 'cert' in p.name)
                    content = cache.read_text()
                    chain = ''.join(re.findall(r'-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----\s*', content, re.S))
                    private = re.search(r'-----BEGIN PRIVATE KEY-----.*?-----END PRIVATE KEY-----\s*', content, re.S)[0]
                    cert, key = root / 'cert.pem', root / 'key.pem'
                    cert.write_text(chain); key.write_text(private)
                    await stop(server)
                    pem_config = root / 'pem.toml'
                    pem_config.write_text(f"listen='127.0.0.1:{public}'\nadmin_listen='127.0.0.1:{admin}'\n"
                        "state_dir='pem-state'\nshutdown_grace_seconds=1\n[tls]\nmode='pem'\ncert='cert.pem'\nkey='key.pem'\n" + route, encoding='utf-8')
                    server = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(pem_config), stdout=server_log, stderr=server_log)
                    await wait_ready(admin, server)
                    current = await certificate(public, trust)
                    checks.append('PEM-HTTPS')
                    # A second explicitly trusted local certificate proves that
                    # successful reload replaces the certificate without restarting.
                    replacement_key = ec.generate_private_key(ec.SECP256R1())
                    subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'localhost')])
                    now = datetime.now(timezone.utc)
                    replacement = x509.CertificateBuilder().subject_name(subject).issuer_name(subject).public_key(replacement_key.public_key()).serial_number(x509.random_serial_number()).not_valid_before(now - timedelta(minutes=1)).not_valid_after(now + timedelta(days=1)).add_extension(x509.SubjectAlternativeName([x509.DNSName('localhost')]), critical=False).sign(replacement_key, hashes.SHA256())
                    replacement_pem = replacement.public_bytes(serialization.Encoding.PEM).decode()
                    trusted_replacement = ssl.create_default_context(cadata=ca_root + '\n' + replacement_pem)
                    cert.write_text(replacement_pem)
                    key.write_bytes(replacement_key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
                    await eventually_certificate(public, trusted_replacement, previous=current['serialNumber'], timeout=15)
                    checks.append('valid-PEM-reloads-without-server-restart')
                    # Invalid replacements must never displace the serving certificate.
                    cert.write_text('invalid replacement')
                    await asyncio.sleep(6)
                    await certificate(public, trusted_replacement)
                    cert.write_text(replacement_pem)
                    checks.append('invalid-PEM-retains-last-good-certificate')
                    await stop(server)
                    legacy_config = root / 'shiny-server.conf'
                    legacy_config.write_text(f'server {{ listen {public} 127.0.0.1; ssl key.pem cert.pem; location / {{ site_dir site; }} }}\n')
                    server = await asyncio.create_subprocess_exec(str(binary.resolve()), 'serve', '--config', str(legacy_config), stdout=server_log, stderr=server_log)
                    await wait_ready(3839, server)
                    await certificate(public, trusted_replacement)
                    checks.append('legacy-ssl-directive-serves-trusted-HTTPS')
            finally:
                await stop(server)
                await stop(ca)
                dns.close()
                dns_tcp.close()
                await dns_tcp.wait_closed()
    report = {'suite': 'local-ACME-and-TLS', 'passed': True, 'checks': checks,
              'public_CA_tested': False, 'certificate_verification_disabled': False}
    (results / 'tls.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    return report

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--pebble', required=True, type=Path)
    parser.add_argument('--certificates', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(asyncio.run(main(args.binary, args.pebble, args.certificates)), indent=2))
