"""Bind the release binary to the delivery checkpoint protocol before execution."""
import hashlib
import json
import os
from pathlib import Path
import re
import sys

PROTOCOL = 1


def digest(binary):
    with Path(binary).open('rb') as handle:
        return hashlib.file_digest(handle, 'sha256').hexdigest()


def create(binary, manifest, source_sha):
    if not re.fullmatch(r'[0-9a-f]{40}', source_sha):
        raise ValueError('Invalid release source commit')
    Path(manifest).write_text(json.dumps({'protocol': PROTOCOL, 'source_sha': source_sha, 'sha256': digest(binary)}, sort_keys=True) + '\n')


def verify(binary, manifest):
    record = json.loads(Path(manifest).read_text())
    if record.get('protocol') != PROTOCOL:
        raise ValueError('Release does not support the required delivery checkpoint protocol')
    if not re.fullmatch(r'[0-9a-f]{40}', record.get('source_sha', '')):
        raise ValueError('Invalid release source commit')
    if not re.fullmatch(r'[0-9a-f]{64}', record.get('sha256', '')) or record['sha256'] != digest(binary):
        raise ValueError('Release binary and manifest do not match; wait for release completion')


if __name__ == '__main__':
    operation, binary, manifest = sys.argv[1:]
    if operation == 'create':
        create(binary, manifest, os.environ['GITHUB_SHA'])
    elif operation == 'verify':
        verify(binary, manifest)
    else:
        raise SystemExit('Unknown manifest operation')
