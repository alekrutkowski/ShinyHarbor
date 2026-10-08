#!/usr/bin/env python3
"""Fail closed for stable releases. A prerelease is never a compatibility claim."""
from __future__ import annotations
import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path
import tomllib
ROOT = Path(__file__).resolve().parents[1]


def problems(root: Path, tag: str) -> list[str]:
    version = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    errors = []
    if tag != "v" + version:
        errors.append(f"Tag {tag!r} does not match Cargo version v{version}")
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", tag):
        errors.append("Invalid release tag")
    if "-" in tag:
        return errors
    if not (root / "Cargo.lock").is_file():
        errors.append("Stable release requires a reviewed, committed Cargo.lock")
    manifest = json.loads((root / "conformance/required.json").read_text(encoding="utf-8"))
    if manifest["certification"] != "verified":
        errors.append("Full Shiny Server compatibility has NOT been verified")
    unimplemented = [c["id"] for c in manifest["cases"] if not c["implemented"]]
    if unimplemented:
        errors.append(f"{len(unimplemented)} mandatory conformance cases do not yet have implementations")
    # Stable evidence is an actual archive plus a matching source-tree digest.
    proof_path = root / "conformance" / "certification.json"
    if not proof_path.is_file():
        errors.append("Missing independently reviewed conformance/certification.json")
        return errors
    proof = json.loads(proof_path.read_text(encoding="utf-8"))
    if proof.get("source_sha256") != source_digest(root):
        errors.append("Conformance evidence is not bound to this source tree")
    required = {c["id"] for c in manifest["cases"]}
    passed = {r["id"] for r in proof.get("results", []) if r.get("result") == "pass" and r.get("artifact_sha256")}
    if not required <= passed:
        errors.append("Mandatory conformance results are missing or did not pass")
    if not proof.get("reviewer") or not proof.get("upstream_commit") or not proof.get("shiny_version"):
        errors.append("Evidence requires an identified reviewer and exact upstream/Shiny versions")
    for result in proof.get("results", []):
        artifact = root / "conformance" / "evidence" / result.get("artifact", "")
        evidence_root = (root / "conformance" / "evidence").resolve()
        if not artifact.resolve().is_relative_to(evidence_root) or not artifact.is_file():
            errors.append(f"Missing or invalid evidence artifact: {result.get('id')}")
        elif hashlib.sha256(artifact.read_bytes()).hexdigest() != result.get("artifact_sha256"):
            errors.append(f"Evidence checksum mismatch: {result.get('id')}")
    return errors


def source_digest(root: Path) -> str:
    digest = hashlib.sha256()
    files = [root / "Cargo.toml", root / "Cargo.lock"]
    files += [p for name in ["src", "r", "assets", "tests", "tools"] for p in (root / name).rglob("*")
              if p.is_file() and "__pycache__" not in p.parts]
    for path in sorted(files):
        if path.is_file():
            digest.update(path.relative_to(root).as_posix().encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", required=True)
    args = parser.parse_args()
    errors = problems(ROOT, args.tag)
    if errors:
        raise SystemExit("RELEASE BLOCKED\n" + "\n".join(f"- {e}" for e in errors))
    print("Prerelease policy passed; compatibility is NOT certified." if "-" in args.tag else "Stable release evidence policy passed. Independent review remains required.")
