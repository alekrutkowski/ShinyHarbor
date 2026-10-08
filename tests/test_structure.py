"""Checks that do not pretend to compile Rust or certify protocol behavior."""
import importlib.util
import hashlib
import json
import re
import tempfile
import unittest
from html.parser import HTMLParser
from pathlib import Path
import tomllib
try:
    import yaml
except ImportError:
    yaml = None
ROOT = Path(__file__).resolve().parents[1]


def load_tool(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'tools' / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Structure(unittest.TestCase):
    def test_cargo_manifest_and_author(self):
        cargo = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding="utf-8"))
        self.assertEqual(cargo['package']['license'], 'MIT')
        self.assertIn('Alek Rutkowski <alek.rutkowski@gmail.com>', cargo['package']['authors'])
        self.assertEqual(cargo['package']['edition'], '2024')

    def test_eight_unique_native_targets(self):
        rows = json.loads((ROOT / 'tools/targets.json').read_text(encoding="utf-8"))['include']
        self.assertEqual(len(rows), 8)
        self.assertEqual(len({r['target'] for r in rows}), 8)
        self.assertEqual(len([r for r in rows if 'windows' in r['target']]), 2)
        self.assertTrue(any(r['runner'] == 'windows-11-arm' for r in rows))

    @unittest.skipUnless(yaml, 'Install PyYAML to validate workflow syntax (required in CI)')
    def test_workflows_parse_and_call_existing_scripts(self):
        for path in (ROOT / '.github/workflows').glob('*.yml'):
            raw = path.read_text(encoding="utf-8")
            workflow = yaml.safe_load(raw)
            self.assertIn('jobs', workflow, path)
            for script in re.findall(r'(?:python|python3)\s+((?:tests|tools)/[\w.-]+\.py)', raw):
                self.assertTrue((ROOT / script).is_file(), script)
            self.assertNotIn('pull_request_target', raw)

    def test_embedded_files_exist(self):
        for source in (ROOT / 'src').glob('*.rs'):
            for name in re.findall(r'include_str!\("([^"]+)"\)', source.read_text(encoding="utf-8")):
                self.assertTrue((source.parent / name).is_file(), (source, name))

    def test_dashboard_ids_match_script(self):
        class Ids(HTMLParser):
            def __init__(self):
                super().__init__(); self.ids = []
            def handle_starttag(self, tag, attrs):
                self.ids += [v for k, v in attrs if k == 'id']
        parser = Ids(); parser.feed((ROOT / 'assets/index.html').read_text(encoding="utf-8"))
        self.assertEqual(len(parser.ids), len(set(parser.ids)))
        used = re.findall(r"\$\('([^']+)'\)", (ROOT / 'assets/app.js').read_text(encoding="utf-8"))
        self.assertTrue(set(used) <= set(parser.ids), set(used) - set(parser.ids))

    def test_no_remote_frontend_or_persistent_token(self):
        html = (ROOT / 'assets/index.html').read_text(encoding="utf-8")
        js = (ROOT / 'assets/app.js').read_text(encoding="utf-8")
        self.assertNotRegex(html, r'(?:src|href)="https?://')
        self.assertNotIn('localStorage', js)
        self.assertNotIn('sessionStorage', js)
        self.assertNotIn('.innerHTML', js)

    def test_conformance_cannot_claim_success(self):
        data = json.loads((ROOT / 'conformance/required.json').read_text(encoding="utf-8"))
        self.assertEqual(data['certification'], 'unverified')
        self.assertTrue(all(not case['implemented'] for case in data['cases']))
        self.assertEqual(len({c['id'] for c in data['cases']}), len(data['cases']))
        self.assertGreater(len(data['cases']), 60)

    def test_prerelease_gate(self):
        gate = load_tool('release_gate')
        version = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding="utf-8"))['package']['version']
        self.assertEqual(gate.problems(ROOT, 'v' + version), [])
        self.assertTrue(gate.problems(ROOT, 'v9.9.9'))

    def test_stable_gate_fails_even_with_matching_version(self):
        gate = load_tool('release_gate')
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); (root / 'conformance').mkdir()
            (root / 'Cargo.toml').write_text('[package]\nversion="1.0.0"\n')
            (root / 'conformance/required.json').write_bytes((ROOT / 'conformance/required.json').read_bytes())
            errors = gate.problems(root, 'v1.0.0')
            self.assertTrue(any('NOT been verified' in e for e in errors))
            self.assertTrue(any('Cargo.lock' in e for e in errors))

    def test_checksum_manifest_requires_all_platforms(self):
        tool = load_tool('release_files')
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(FileNotFoundError):
                tool.verify(Path(temp))

    def test_complete_checksum_manifest_and_tamper_detection(self):
        tool = load_tool('release_files')
        version = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding="utf-8"))['package']['version']
        targets = json.loads((ROOT / 'tools/targets.json').read_text(encoding="utf-8"))['include']
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            files = []
            for row in targets:
                ext = 'zip' if 'windows' in row['target'] else 'tar.gz'
                path = directory / f"shinyharbor-{version}-{row['target']}.{ext}"
                path.write_bytes(b'Synthetic artifact used only to test digest verification')
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                path.with_name(path.name + '.sha256').write_text(f'{digest}  {path.name}\n')
                files.append(path)
            self.assertEqual(len(tool.verify(directory)), 8)
            self.assertEqual(len((directory / 'SHA256SUMS').read_text(encoding="utf-8").splitlines()), 8)
            files[0].write_bytes(b'tampered')
            with self.assertRaises(ValueError):
                tool.verify(directory)

    def test_examples_are_valid_toml(self):
        for path in (ROOT / 'examples').glob('*.toml'):
            config = tomllib.loads(path.read_text(encoding="utf-8"))
            self.assertIn('listen', config, path)
            self.assertTrue(config.get('apps') or config.get('upstreams') or config.get('apps_dir') or config.get('sites'), path)

    def test_documented_status(self):
        readme = (ROOT / 'README.md').read_text(encoding="utf-8")
        self.assertIn('not a certified drop-in replacement', readme)
        self.assertTrue((ROOT / 'docs/COMPATIBILITY.md').is_file())
        self.assertTrue((ROOT / 'SECURITY.md').is_file())

if __name__ == '__main__':
    unittest.main()
