#!/usr/bin/env python3
"""Fail an incomplete release, then emit a consolidated checksum manifest."""
from pathlib import Path
import argparse
import hashlib
import json
import tomllib

ROOT = Path(__file__).resolve().parents[1]

def verify(directory: Path) -> list[str]:
    targets = json.loads((ROOT / "tools/targets.json").read_text(encoding="utf-8"))["include"]
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    lines = []
    for row in targets:
        target = row["target"]
        ext = "zip" if "windows" in target else "tar.gz"
        file = directory / f"shinyharbor-{version}-{target}.{ext}"
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
