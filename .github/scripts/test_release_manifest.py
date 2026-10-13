import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('release_manifest', Path(__file__).with_name('release_manifest.py'))
manifest = importlib.util.module_from_spec(spec)
spec.loader.exec_module(manifest)

class ReleaseManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.binary = Path(self.temp.name) / 'bot'
        self.record = Path(self.temp.name) / 'manifest.json'
        self.binary.write_bytes(b'binary-with-pending-protocol')
        manifest.create(self.binary, self.record, 'a' * 40)

    def test_matching_protocol_and_hash_pass(self):
        manifest.verify(self.binary, self.record)

    def test_old_binary_or_racing_asset_update_fails(self):
        self.binary.write_bytes(b'old-or-different-binary')
        with self.assertRaises(ValueError): manifest.verify(self.binary, self.record)

    def test_unsupported_protocol_fails(self):
        record = json.loads(self.record.read_text())
        record['protocol'] = 0
        self.record.write_text(json.dumps(record))
        with self.assertRaises(ValueError): manifest.verify(self.binary, self.record)

    def test_missing_manifest_fails(self):
        self.record.unlink()
        with self.assertRaises(FileNotFoundError): manifest.verify(self.binary, self.record)

    def test_malformed_manifest_fails(self):
        self.record.write_text('{}')
        with self.assertRaises(ValueError): manifest.verify(self.binary, self.record)

    def test_invalid_source_commit_fails(self):
        with self.assertRaises(ValueError): manifest.create(self.binary, self.record, 'main')

if __name__ == '__main__': unittest.main()
