#!/usr/bin/env python3
"""Create an archive, dependency notices, an SBOM and a SHA-256 sidecar.

Run after cargo metadata and cargo build with the same lockfile and target.
Only the supplied compiled binary is packaged; no R or upstream Shiny Server.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import re
import shutil
import subprocess
import tarfile
import tempfile
import zipfile
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def metadata() -> dict:
    return json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--format-version", "1"
    ], cwd=ROOT, text=True))


def dependencies(meta: dict, destination: Path) -> None:
    """Retain all resolved crates, including build dependencies, conservatively."""
    out = destination / "licenses"
    out.mkdir()
    notices = ["# Resolved dependency acknowledgments", "",
               "Generated from Cargo metadata for this build. Includes build/dev dependencies conservatively.",
               "License texts are retained in licenses/. The source project's MIT license does not relicense dependencies.", ""]
    components = []
    supplemental_root = ROOT / 'assets/licenses'
    supplemental = json.loads((supplemental_root / 'manifest.json').read_text(encoding='utf-8-sig'))
    for package in sorted(meta["packages"], key=lambda p: (p["name"], p["version"])):
        if package["name"] == "shinyharbor":
            continue
        key = f'{package["name"]}-{package["version"]}'
        folder = out / key
        folder.mkdir()
        root = Path(package["manifest_path"]).parent
        candidates = [p for p in root.rglob("*") if p.is_file()
                      and re.match(r"^(licen[sc]e|copying|notice|copyright)([._-].*)?$", p.name, re.I)]
        # Some projects (notably r-efi) put their full license in AUTHORS.
        authors_file = root / 'AUTHORS'
        if authors_file.is_file() and 'Permission is hereby granted' in authors_file.read_text(encoding='utf-8'):
            candidates.append(authors_file)
        explicit = package.get("license_file")
        if explicit and (root / explicit).is_file():
            candidates.append(root / explicit)
        additions = [entry for entry in supplemental if entry['package'] == key]
        for entry in additions:
            notice = supplemental_root / entry['file']
            if hashlib.sha256(notice.read_bytes()).hexdigest() != entry['sha256']:
                raise RuntimeError(f'Supplemental license digest mismatch: {key}')
            shutil.copyfile(notice, folder / entry['file'])
        if additions:
            (folder / 'sources.json').write_text(json.dumps(additions, indent=2), encoding='utf-8')
        if any(entry.get('standard_text') for entry in additions):
            # These two crates explicitly offer Apache-2.0 OR MIT but publish no
            # license file. Retain their actual declaration and source notices,
            # alongside the selected standard Apache license, without inventing
            # a copyright holder or presenting it as an upstream notice.
            shutil.copyfile(root / 'Cargo.toml.orig', folder / 'UPSTREAM-Cargo.toml')
            shutil.copytree(root / 'src', folder / 'upstream-source-notices')
            (folder / 'NOTICE.txt').write_text(
                'This distribution selects the Apache-2.0 option declared in the retained upstream Cargo manifest. '
                'Upstream did not supply a standalone license file. Apache-2.0.txt is the standard license text; '
                'the original source is retained for any embedded notices. No additional copyright notice has been invented.\n', encoding='utf-8')
        if not candidates and not additions:
            raise RuntimeError(f"No license text found for {key}; review before distribution")
        for index, path in enumerate(sorted(set(candidates))):
            shutil.copyfile(path, folder / f"{index:02d}-{path.name}")
        license_expression = (package.get("license") or "LicenseRef-review-required").replace('MIT/Apache-2.0', 'MIT OR Apache-2.0')
        notices += [f"## {key}", f"License: `{license_expression}`",
                    f"Authors: {', '.join(package.get('authors', [])) or 'See retained notices'}",
                    f"Source: {package.get('repository') or package.get('source') or 'See Cargo metadata'}", ""]
        components.append({"type": "library", "name": package["name"], "version": package["version"],
                           "purl": f'pkg:cargo/{package["name"]}@{package["version"]}',
                           "licenses": [{"expression": license_expression}]})
    sockjs = out / "sockjs-client-1.6.1"
    sockjs.mkdir()
    shutil.copyfile(ROOT / "assets/vendor/sockjs-LICENSE", sockjs / "LICENSE")
    notices += ["## sockjs-client 1.6.1", "License: `MIT`",
                "Source: https://github.com/sockjs/sockjs-client/tree/v1.6.1", ""]
    components.append({"type": "library", "name": "sockjs-client", "version": "1.6.1",
                       "purl": "pkg:npm/sockjs-client@1.6.1", "licenses": [{"expression": "MIT"}]})
    (destination / "THIRD_PARTY.md").write_text("\n".join(notices), encoding="utf-8")
    (destination / "sbom.cdx.json").write_text(json.dumps({
        "bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1, "components": components
    }, indent=2), encoding="utf-8")
    # Metadata retains dependency identities and resolution graph, not private filesystem paths.
    for package in meta["packages"]:
        package.pop("manifest_path", None)
        package.pop("targets", None)
        if package.get('license_file'):
            package['license_file'] = Path(package['license_file']).name
    for field in ["workspace_root", "target_directory", "build_directory"]:
        meta.pop(field, None)
    encoded = json.dumps(meta, indent=2)
    for package in meta['packages']:
        if package['source'] is None:
            encoded = encoded.replace(json.dumps(package['id']), json.dumps(f"{package['name']}@{package['version']}"))
    (destination / "cargo-metadata.json").write_text(encoded, encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True)
    parser.add_argument("--binary", type=Path, help="Use an executable from a custom Cargo target directory")
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    supported = {r["target"] for r in json.loads((ROOT / "tools/targets.json").read_text(encoding="utf-8"))["include"]}
    if args.target not in supported:
        raise SystemExit("Target is not in tools/targets.json")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    binary_name = "shinyharbor.exe" if "windows" in args.target else "shinyharbor"
    binary = args.binary or ROOT / "target" / args.target / "release" / binary_name
    if not binary.is_file():
        raise SystemExit(f"Build artifact is missing: {binary}")
    args.output.mkdir(parents=True, exist_ok=True)
    stem = f"shinyharbor-{version}-{args.target}"
    with tempfile.TemporaryDirectory() as temp:
        folder = Path(temp) / stem
        folder.mkdir()
        shutil.copy2(binary, folder / binary_name)
        shutil.copy2(ROOT / 'examples/init.toml', folder / 'harbor.toml')
        shutil.copytree(ROOT / 'examples/hello', folder / 'apps/hello')
        shutil.copytree(ROOT / 'assets/brand', folder / 'brand')
        for file in ["LICENSE", "README.md", "Cargo.lock", "ATTRIBUTIONS.md", "SECURITY.md", "VALIDATION.md"]:
            shutil.copy2(ROOT / file, folder / file)
        for directory in ["docs", "examples", "packaging", "conformance", "validation"]:
            shutil.copytree(ROOT / directory, folder / directory, ignore=shutil.ignore_patterns("evidence"))
        dependencies(metadata(), folder)
        if "windows" in args.target:
            archive = args.output / f"{stem}.zip"
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as target:
                for path in sorted(folder.rglob("*")):
                    if path.is_file():
                        target.write(path, path.relative_to(Path(temp)))
        else:
            archive = args.output / f"{stem}.tar.gz"
            with tarfile.open(archive, "w:gz") as target:
                def modes(member):
                    member.uid = member.gid = 0
                    member.uname = member.gname = 'root'
                    member.mode = 0o755 if member.isdir() or member.name == f'{stem}/shinyharbor' or member.name.endswith('.sh') else 0o644
                    return member
                target.add(folder, arcname=stem, filter=modes)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_name(archive.name + ".sha256").write_text(f"{digest}  {archive.name}\n", encoding="ascii")
    print(archive)


if __name__ == "__main__":
    main()
