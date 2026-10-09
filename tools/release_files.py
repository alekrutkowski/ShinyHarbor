#!/usr/bin/env python3
"""Fail an incomplete release, then emit a consolidated checksum manifest."""
from pathlib import Path
import argparse
import hashlib
import json
import tomllib

ROOT = Path(__file__).resolve().parents[1]

def expected_files(version: str, targets: list[dict]) -> list[str]:
    files = []
    for row in targets:
        target = row['target']
        stem = f'shinyharbor-{version}-{target}'
        files.append(stem + ('.zip' if 'windows' in target else '.tar.gz'))
        if 'windows' in target:
            files.append(stem + '-setup.exe')
        elif 'apple' in target:
            files.append(stem + '.pkg')
        elif row['musl']:
            architecture = 'aarch64' if target.startswith('aarch64') else 'x86_64'
            deb_arch = 'arm64' if architecture == 'aarch64' else 'amd64'
            native = version.replace('-', '~', 1)
            files += [f'shinyharbor_{native}-1_{deb_arch}.deb',
                      f'shinyharbor-{native}-1.{architecture}.rpm',
                      f'shinyharbor-{version.replace("-", "_")}-1-{architecture}.pkg.tar.zst']
    return files

def verify(directory: Path) -> list[str]:
    targets = json.loads((ROOT / "tools/targets.json").read_text(encoding="utf-8"))["include"]
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    lines = []
    for name in expected_files(version, targets):
        file = directory / name
        digest = hashlib.sha256(file.read_bytes()).hexdigest()
        line = f"{digest}  {file.name}"
        if file.with_name(file.name + ".sha256").read_text(encoding="utf-8").strip() != line:
            raise ValueError(f"Checksum mismatch: {file}")
        lines.append(line)
    (directory / "SHA256SUMS").write_text("\n".join(lines) + "\n", encoding="ascii")
    return lines

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", type=Path, required=True)
    verify(parser.parse_args().directory)
