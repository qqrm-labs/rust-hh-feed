"""Durable pending/confirmed delivery state using GitHub's contents API."""
import base64
import hashlib
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

BRANCH = 'delivery-state'
BOOTSTRAP = 'confirmed-absent'


def expected_path(local_file):
    return Path(str(local_file) + '.remote-sha')


def remember(local_file, sha):
    if not isinstance(sha, str) or not sha:
        raise RuntimeError('Checkpoint response did not identify the committed blob')
    path = expected_path(local_file)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(str(path) + '.tmp')
    temporary.write_text(sha)
    temporary.replace(path)


def api(endpoint, method='GET', payload=None):
    request = urllib.request.Request(
        'https://api.github.com/repos/' + os.environ['GITHUB_REPOSITORY'] + endpoint,
        data=None if payload is None else json.dumps(payload).encode(), method=method,
        headers={'Authorization': 'Bearer ' + os.environ['GH_TOKEN'],
                 'Accept': 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28'},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        if error.code == 404 and method == 'GET':
            return None
        raise RuntimeError(f'Checkpoint API failed: HTTP {error.code}') from None
    except urllib.error.URLError:
        raise RuntimeError('Checkpoint API connection failed') from None


def decode(remote):
    raw = base64.b64decode(''.join(remote['content'].split()), validate=True)
    envelope = json.loads(raw)
    if envelope.get('version') != 1 or not isinstance(envelope.get('pending'), (str, type(None))):
        raise RuntimeError('Invalid delivery checkpoint envelope')
    base64.b64decode(envelope['state'], validate=True)
    return envelope


def main():
    operation, local_file, *arguments = sys.argv[1:]
    key = os.environ['DELIVERY_STATE_KEY']
    if '/' in key or key in ('', '.', '..'):
        raise RuntimeError('Checkpoint key must be a filename')
    endpoint = '/contents/' + urllib.parse.quote(key, safe='')
    remote = api(endpoint + '?ref=' + BRANCH)
    envelope = None if remote is None else decode(remote)
    if envelope and envelope['pending'] and operation != 'confirm':
        raise RuntimeError('Unconfirmed delivery pending; reconcile delivery-state before retrying')
    if operation == 'restore':
        if remote is None:
            if api('/git/ref/heads/' + BRANCH) is not None:
                raise RuntimeError('Existing delivery-state branch has no checkpoint')
            if api('') is None:
                raise RuntimeError('Repository unavailable; cannot establish first state')
            remember(local_file, BOOTSTRAP)
            sys.exit(3)
        content = base64.b64decode(envelope['state'], validate=True)
        path = Path(local_file)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        remember(local_file, remote['sha'])
        return
    if operation not in ('save', 'begin', 'confirm'):
        raise RuntimeError('Unknown checkpoint operation')
    expected = expected_path(local_file).read_text()
    if (remote is None and expected != BOOTSTRAP) or (remote is not None and remote['sha'] != expected):
        raise RuntimeError('Delivery checkpoint changed since restore; refusing stale state')
    if operation in ('begin', 'confirm') and envelope is None:
        raise RuntimeError('Delivery must establish initial state before sending')
    if operation == 'confirm' and not envelope['pending']:
        raise RuntimeError('No pending delivery to confirm')
    if remote is None:
        if api('/git/ref/heads/' + BRANCH) is not None:
            raise RuntimeError('Existing delivery-state branch has no checkpoint')
        repository = api('')
        head = api('/git/ref/heads/' + repository['default_branch'])
        api('/git/refs', 'POST', {'ref': 'refs/heads/' + BRANCH, 'sha': head['object']['sha']})
    next_state = {'version': 1, 'state': base64.b64encode(Path(local_file).read_bytes()).decode(),
                  'pending': hashlib.sha256(arguments[0].encode()).hexdigest() if operation == 'begin' else None}
    if envelope == next_state:
        return
    payload = {'message': 'Checkpoint delivery progress', 'branch': BRANCH,
               'content': base64.b64encode(json.dumps(next_state, sort_keys=True).encode()).decode()}
    if remote is not None:
        payload['sha'] = remote['sha']
    response = api(endpoint, 'PUT', payload)
    remember(local_file, response.get('content', {}).get('sha'))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(f'Delivery checkpoint failed: {error}', file=sys.stderr)
        sys.exit(1)
