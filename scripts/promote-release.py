#!/usr/bin/env python3
"""Verify a release and print an approval token; --approval TOKEN publishes its draft.

The token binds repository, tag, commit, release ID, asset IDs and verified bytes.
No assets are uploaded, replaced or deleted. Retry after an uncertain publish
response re-verifies the same published bytes instead of publishing again.
The token is a review fingerprint, not authentication: run this trusted local
controller with an owner-authorized write credential; never execute candidate code.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile


def gh(*args, output=None, size=None):
    if output is not None:
        # Bound actual response bytes, not just the API's claimed asset size.
        with tempfile.TemporaryFile() as errors:
            with subprocess.Popen(['gh', *args], stdout=subprocess.PIPE, stderr=errors) as process:
                try:
                    remaining = size
                    while chunk := process.stdout.read(min(65536, remaining + 1)):
                        if len(chunk) > remaining:
                            raise ValueError('download exceeds declared asset size')
                        output.write(chunk)
                        remaining -= len(chunk)
                    process.wait()
                except BaseException:
                    process.kill()
                    process.wait()
                    raise
                if process.returncode:
                    errors.seek(0)
                    raise RuntimeError(errors.read(65536).decode(errors='replace').strip())
        return None
    result = subprocess.run(['gh', *args], stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE)
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors='replace').strip())
    return json.loads(result.stdout)


def verify(repo, tag, commit, directory):
    prefix = f'repos/{repo}'
    if gh('api', f'{prefix}/commits/refs/tags/{tag}')['sha'] != commit:
        raise ValueError('tag does not resolve to approved commit')
    release = gh('api', f'{prefix}/releases/tags/{tag}')
    if release['tag_name'] != tag or release['prerelease']:
        raise ValueError('wrong tag or prerelease')
    assets = release['assets']
    if len(assets) != 11 or len({a['name'] for a in assets}) != 11:
        raise ValueError('release requires exactly 11 unique assets')
    hashes = {}
    for asset in assets:
        name = asset['name']
        if not re.fullmatch(r'[A-Za-z0-9_.-]+', name):
            raise ValueError('invalid asset name')
        maximum = 16 * 1024 * 1024 if name.endswith('.json') else 128 * 1024 * 1024
        if type(asset['size']) is not int or not 0 < asset['size'] <= maximum:
            raise ValueError('invalid asset size')
        path = directory / name
        with path.open('wb') as stream:
            gh('api', f'{prefix}/releases/assets/{asset["id"]}',
               '-H', 'Accept: application/octet-stream', output=stream, size=asset['size'])
        if path.stat().st_size != asset['size']:
            raise ValueError('downloaded asset size mismatch')
        hashes[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    subprocess.run(['python3', str(Path(__file__).with_name('verify-release-bundle.py')),
                    str(directory), tag, commit], check=True)
    identity = {'repo': repo, 'tag': tag, 'commit': commit, 'id': release['id'],
                'assets': sorted((a['id'], a['name'], a['size'], a['updated_at'])
                                 for a in assets), 'hashes': hashes}
    token = hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()
    # ponytail: cooperative release writers only; GitHub offers no CAS across
    # asset/tag verification and publish. Freeze manual tag/asset edits during promotion.
    current = gh('api', f'{prefix}/releases/{release["id"]}')
    if (current['id'] != release['id'] or current['tag_name'] != tag
            or current['prerelease']
            or sorted((a['id'], a['name'], a['size'], a['updated_at'])
                      for a in current['assets']) != identity['assets']
            or current['draft'] != release['draft']
            or gh('api', f'{prefix}/commits/refs/tags/{tag}')['sha'] != commit):
        raise ValueError('release changed while verifying')
    return release, token


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('repo')
    parser.add_argument('tag')
    parser.add_argument('commit')
    parser.add_argument('--approval', help='exact token from an owner-reviewed verification')
    args = parser.parse_args()
    if (not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', args.repo)
            or not re.fullmatch(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', args.tag)
            or not re.fullmatch(r'[0-9a-f]{40}', args.commit)):
        parser.error('invalid repository, stable tag or commit')
    with tempfile.TemporaryDirectory(prefix='agentfoundry-promote-') as temp:
        release, token = verify(args.repo, args.tag, args.commit, Path(temp))
    if args.approval is None:
        print(json.dumps({'release_id': release['id'], 'draft': release['draft'],
                          'approval_token': token, 'published': False}))
        return
    if args.approval != token:
        raise ValueError('approval token does not match verified release bytes/identity')
    if not release['draft']:
        print('Already published; approved bytes verified, no mutation')
        return
    endpoint = f'repos/{args.repo}/releases/{release["id"]}'
    try:
        gh('api', '--method', 'PATCH', endpoint, '-F', 'draft=false')
    except (RuntimeError, json.JSONDecodeError):
        # A lost response is not proof of failure or permission to retry a write.
        with tempfile.TemporaryDirectory(prefix='agentfoundry-promote-reconcile-') as temp:
            reconciled, actual = verify(args.repo, args.tag, args.commit, Path(temp))
        if reconciled['draft'] or actual != token:
            raise
    with tempfile.TemporaryDirectory(prefix='agentfoundry-promote-final-') as temp:
        published, actual = verify(args.repo, args.tag, args.commit, Path(temp))
    if published['draft'] or actual != token:
        raise ValueError('publication state/bytes differ; inspect release, do not overwrite')
    print('Published approved complete release; bytes verified')


if __name__ == '__main__':
    main()
