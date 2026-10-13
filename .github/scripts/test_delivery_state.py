"""Failure/recovery contracts; all GitHub calls are mocked."""
import base64
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import urllib.error

spec = importlib.util.spec_from_file_location('delivery_state', Path(__file__).with_name('delivery_state.py'))
state = importlib.util.module_from_spec(spec)
spec.loader.exec_module(state)

def remote(content=b'confirmed', pending=None):
    envelope = {'version': 1, 'state': base64.b64encode(content).decode(), 'pending': pending}
    return {'content': base64.b64encode(json.dumps(envelope).encode()).decode(), 'sha': 'blob-sha'}

class CheckpointTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'checkpoint.json'
        self.env = patch.dict(os.environ, {'DELIVERY_STATE_KEY': 'checkpoint.json', 'GITHUB_REPOSITORY': 'test/repo', 'GH_TOKEN': 'TEST'})
        self.env.start()
        self.addCleanup(self.env.stop)
        state.remember(self.path, 'blob-sha')

    def run_operation(self, operation, *args):
        with patch.object(sys, 'argv', ['delivery_state.py', operation, str(self.path), *args]):
            state.main()

    def test_only_missing_branch_allows_bootstrap(self):
        with patch.object(state, 'api', side_effect=[None, None, {'default_branch': 'main'}]):
            with self.assertRaises(SystemExit) as result:
                self.run_operation('restore')
            self.assertEqual(result.exception.code, 3)
        for replies in ([None, {'object': {'sha': 'existing'}}], [None, None, None]):
            with patch.object(state, 'api', side_effect=replies):
                with self.assertRaises(RuntimeError): self.run_operation('restore')
        self.assertFalse(self.path.exists())

    def test_auth_and_network_errors_do_not_become_empty_state(self):
        errors = [urllib.error.HTTPError('https://example.invalid', 401, 'denied', {}, None),
                  urllib.error.HTTPError('https://example.invalid', 403, 'denied', {}, None),
                  urllib.error.URLError('offline')]
        for error in errors:
            with self.subTest(error=type(error).__name__), patch.object(state.urllib.request, 'urlopen', side_effect=error):
                with self.assertRaises(RuntimeError): self.run_operation('restore')
                self.assertFalse(self.path.exists())

    def test_pending_is_durable_before_send_and_blocks_crash_retry(self):
        self.path.write_bytes(b'confirmed')
        with patch.object(state, 'api', side_effect=[remote(), {'content': {'sha': 'pending-sha'}}]) as api:
            self.run_operation('begin', 'item-2')
            payload = api.call_args.args[2]
            pending = state.decode({'content': payload['content']})
            self.assertEqual(payload['sha'], 'blob-sha')
            self.assertEqual(len(pending['pending']), 64)
            self.assertEqual(base64.b64decode(pending['state']), b'confirmed')
            self.assertEqual(state.expected_path(self.path).read_text(), 'pending-sha')
        for operation in ('restore', 'save', 'begin'):
            with patch.object(state, 'api', return_value=remote(pending=pending['pending'])) as api:
                with self.assertRaises(RuntimeError): self.run_operation(operation, 'item-2')
                self.assertEqual(api.call_count, 1)

    def test_ack_promotes_state_and_clears_pending_in_one_write(self):
        self.path.write_bytes(b'next confirmed')
        with patch.object(state, 'api', side_effect=[remote(pending='pending-hash'), {'content': {'sha': 'confirmed-sha'}}]) as api:
            self.run_operation('confirm')
            payload = api.call_args.args[2]
            updated = state.decode({'content': payload['content']})
            self.assertEqual(payload['sha'], 'blob-sha')
            self.assertIsNone(updated['pending'])
            self.assertEqual(base64.b64decode(updated['state']), b'next confirmed')
            self.assertEqual(state.expected_path(self.path).read_text(), 'confirmed-sha')

    def test_failed_checkpoint_after_ack_leaves_pending_and_blocks_next_run(self):
        self.path.write_bytes(b'next confirmed')
        persistent = remote(pending='pending-hash')
        with patch.object(state, 'api', side_effect=[persistent, RuntimeError('HTTP 409')]):
            with self.assertRaises(RuntimeError): self.run_operation('confirm')
        self.path.unlink()
        with patch.object(state, 'api', return_value=persistent):
            with self.assertRaises(RuntimeError): self.run_operation('restore')
        self.assertFalse(self.path.exists())

    def test_unchanged_state_is_not_committed_and_valid_state_restores(self):
        with patch.object(state, 'api', return_value=remote()): self.run_operation('restore')
        self.assertEqual(self.path.read_bytes(), b'confirmed')
        with patch.object(state, 'api', return_value=remote()) as api:
            self.run_operation('save')
            self.assertEqual(api.call_count, 1)

    def test_corrupt_envelope_is_rejected(self):
        for content in ('not valid base64!!!', base64.b64encode(b'{}').decode()):
            with patch.object(state, 'api', return_value={'content': content}):
                with self.assertRaises((ValueError, RuntimeError)): self.run_operation('restore')
        self.assertFalse(self.path.exists())

    def test_changed_remote_after_restore_never_overwrites_newer_state(self):
        with patch.object(state, 'api', return_value=remote()): self.run_operation('restore')
        self.assertEqual(state.expected_path(self.path).read_text(), 'blob-sha')
        for operation in ('save', 'begin', 'confirm'):
            latest = remote(b'newer confirmed state', pending='another-pending' if operation == 'confirm' else None)
            latest['sha'] = 'newer-sha'
            with self.subTest(operation=operation), patch.object(state, 'api', return_value=latest) as api:
                with self.assertRaisesRegex(RuntimeError, 'changed since restore'):
                    self.run_operation(operation, 'item')
                self.assertEqual(api.call_count, 1)
                self.assertEqual(api.call_args.args, ('/contents/checkpoint.json?ref=delivery-state',))

    def test_missing_restore_receipt_does_not_allow_bootstrap_or_overwrite(self):
        state.expected_path(self.path).unlink()
        self.path.write_bytes(b'local stale state')
        for existing in (None, remote()):
            with patch.object(state, 'api', return_value=existing) as api:
                with self.assertRaises(FileNotFoundError): self.run_operation('save')
                self.assertEqual(api.call_count, 1)

    def test_bootstrap_race_with_another_writer_fails_before_put(self):
        with patch.object(state, 'api', side_effect=[None, None, {'default_branch': 'main'}]):
            with self.assertRaises(SystemExit): self.run_operation('restore')
        self.path.write_bytes(b'initial')
        with patch.object(state, 'api', return_value=remote(b'another runner state')) as api:
            with self.assertRaisesRegex(RuntimeError, 'changed since restore'): self.run_operation('save')
            self.assertEqual(api.call_count, 1)

if __name__ == '__main__': unittest.main()
