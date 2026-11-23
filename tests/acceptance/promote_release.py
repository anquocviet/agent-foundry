#!/usr/bin/env python3
"""Run actual promotion logic/verifier against a deterministic GitHub API boundary."""
import contextlib
import copy
import hashlib
import os
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('promote', ROOT / 'scripts/promote-release.py')
promote = importlib.util.module_from_spec(spec)
spec.loader.exec_module(promote)
TAG, COMMIT, REPO = 'v1.2.3', 'a' * 40, 'owner/repo'
PLATFORMS = ('x86_64-apple-darwin', 'aarch64-apple-darwin',
             'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu', 'x86_64-pc-windows-msvc')

with tempfile.TemporaryDirectory() as temp:
    source = Path(temp) / 'source'
    source.mkdir()
    for platform in PLATFORMS:
        directory = source / ('stable-candidate-' + platform)
        directory.mkdir()
        data = platform.encode()
        name = 'agentfoundry.exe' if platform.endswith('msvc') else 'agentfoundry'
        (directory / name).write_bytes(data)
        (directory / 'identity.json').write_text(json.dumps(dict(
            platform=platform, version=TAG[1:], source=COMMIT, distribution='stable',
            fault_injection=False, sha256=hashlib.sha256(data).hexdigest(), payload_digest='b' * 64)))
    bundle = Path(temp) / 'bundle'
    subprocess.run([sys.executable, str(ROOT / 'scripts/package-stable.py'),
                    str(source), str(bundle), TAG, COMMIT], check=True)
    blobs = {index: p.read_bytes() for index, p in enumerate(sorted(bundle.iterdir()), 1)}
    assets = [dict(id=index, name=p.name, size=p.stat().st_size, updated_at='fixed', download_count=0)
              for index, p in enumerate(sorted(bundle.iterdir()), 1)]
    original = dict(id=42, tag_name=TAG, draft=True, prerelease=False, assets=assets)
    state = {}
    writes = []
    fault = None
    tag_reads = 0

    def api(*args, output=None, size=None):
        global fault, tag_reads
        if '--method' in args:
            writes.append(args)
            if fault == 'before-publish':
                raise RuntimeError('interrupted before server mutation')
            state['draft'] = False
            if fault == 'lost-response':
                raise RuntimeError('response lost after server mutation')
            if fault == 'invalid-response':
                raise json.JSONDecodeError('truncated response after mutation', '', 0)
            return copy.deepcopy(state)
        endpoint = args[1]
        if '/commits/' in endpoint:
            assert endpoint.endswith('/commits/refs/tags/' + TAG), endpoint
            tag_reads += 1
            return {'sha': 'c' * 40 if fault == 'tag-moved' and tag_reads > 1 else COMMIT}
        if '/assets/' in endpoint:
            identifier = int(endpoint.rsplit('/', 1)[1])
            if fault == 'download-count':
                next(a for a in state['assets'] if a['id'] == identifier)['download_count'] += 1
            if fault == 'download':
                raise RuntimeError('interrupted asset verification')
            if fault == 'bytes-changed':
                output.write(b'X' * len(blobs[identifier]))
                return
            output.write(blobs[identifier])
            return
        return copy.deepcopy(state)

    def invoke(token=None):
        argv = ['promote-release.py', REPO, TAG, COMMIT]
        if token is not None:
            argv += ['--approval', token]
        capture = io.StringIO()
        with patch.object(promote, 'gh', api), patch.object(sys, 'argv', argv), contextlib.redirect_stdout(capture):
            promote.main()
        return capture.getvalue()

    def reset():
        global tag_reads
        tag_reads = 0
        state.clear()
        state.update(copy.deepcopy(original))
        writes.clear()

    reset()
    token = json.loads(invoke())['approval_token']
    assert not writes and state['draft'], 'verification published without approval'
    scenarios = ('wrong-approval', 'download', 'partial-inventory', 'before-publish',
                 'lost-response', 'invalid-response', 'tag-moved', 'identity-changed',
                 'bytes-changed', 'download-count', 'success')
    for scenario in scenarios:
        reset()
        fault = scenario
        if scenario == 'partial-inventory':
            state['assets'].pop()
        if scenario == 'identity-changed':
            state['id'] += 1
        try:
            invoke('0' * 64 if scenario == 'wrong-approval' else token)
        except (ValueError, RuntimeError, subprocess.CalledProcessError):
            assert scenario in ('wrong-approval', 'download', 'partial-inventory', 'before-publish',
                                'tag-moved', 'identity-changed', 'bytes-changed'), scenario
            assert state['draft'], 'incomplete release became public'
        else:
            assert scenario in ('lost-response', 'invalid-response', 'download-count', 'success'), scenario
            assert not state['draft']
            assert [{**a, 'download_count': 0} for a in state['assets']] == assets
            if scenario == 'download-count':
                assert all(a['download_count'] > 0 for a in state['assets'])
            prior = len(writes)
            fault = None
            invoke(token)
            assert len(writes) == prior, 'retry rewrote published release'
        if scenario not in ('before-publish', 'lost-response', 'invalid-response', 'download-count', 'success'):
            assert not writes
        print(scenario, 'passed')
    # Exercise the real subprocess download boundary, including dishonest size metadata.
    shim = Path(temp) / 'gh'
    shim.write_text('#!' + sys.executable + '\nimport sys\nsys.stdout.buffer.write(b"x" * 1000000)\n')
    shim.chmod(0o755)
    with patch.dict(os.environ, PATH=temp + os.pathsep + os.environ['PATH']):
        with tempfile.TemporaryFile() as output:
            try:
                promote.gh('api', 'asset', output=output, size=3)
            except ValueError:
                assert output.tell() <= 3, 'unbounded download written to disk'
            else:
                raise AssertionError('oversized response accepted')
    print(f'promotion approval/interruption checks passed: {len(scenarios)} scenarios plus read-only approval and bounded transport')
