"""Install a checksum-pinned local test CA; never added to the system trust store."""
import hashlib
import json
import platform
import shutil
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

def download(item, target):
    with urllib.request.urlopen(item['url'], timeout=120) as response:
        data = response.read(64 * 1024 * 1024)
    if hashlib.sha256(data).hexdigest() != item['sha256']:
        raise RuntimeError(f"Checksum mismatch: {item['url']}")
    target.write_bytes(data)

def main():
    manifest = json.loads((ROOT / 'tools/pebble-assets.json').read_text())
    system = {'Windows': 'windows', 'Linux': 'linux', 'Darwin': 'darwin'}[platform.system()]
    arch = {'amd64':'amd64', 'x86_64':'amd64', 'arm64':'arm64', 'aarch64':'arm64'}[platform.machine().lower()]
    item = manifest['assets'][f'pebble-{system}-{arch}.zip']
    folder = ROOT / '.build/pebble-ci'
    folder.mkdir(parents=True, exist_ok=True)
    archive = folder / 'pebble.zip'
    download(item, archive)
    name = 'pebble.exe' if system == 'windows' else 'pebble'
    # Only extract the executable; paths inside the archive are never trusted.
    with zipfile.ZipFile(archive) as package:
        entries = [f for f in package.infolist() if Path(f.filename).name == name]
        if len(entries) != 1:
            raise RuntimeError('Expected exactly one Pebble executable')
        with package.open(entries[0]) as source, (folder / name).open('wb') as target:
            shutil.copyfileobj(source, target)
    (folder / name).chmod(0o755)
    certs = folder / 'certs'
    certs.mkdir(exist_ok=True)
    for name, item in manifest['certificates'].items():
        download(item, certs / name)
    archive.unlink()
    print(f"Installed Pebble {manifest['version']} for local testing")

if __name__ == '__main__':
    main()
