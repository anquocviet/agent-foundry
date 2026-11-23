#!/usr/bin/env python3
"""T-060 native packaged install/status/doctor/merge/recovery matrix.

Consumes exact packaged artifact bytes on a native host. The fault-injection
build is used only to create crash checkpoints; it is never packaged or used
for install/status/doctor/merge. Writes a machine-readable proof file.

Usage:
  python tests/acceptance/native_matrix.py \
    --platform x86_64-unknown-linux-gnu \
    --binary dist/agentfoundry-x86_64-unknown-linux-gnu \
    --fault-binary target/fault-injection/debug/agentfoundry \
    --output dist/native-matrix-x86_64-unknown-linux-gnu.json \
    [--expected-distribution development|stable] \
    [--source-freeze SOURCE-IDENTITY.json]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import tempfile
import time

PLATFORMS = {
    'x86_64-unknown-linux-gnu': ('linux', 'x86_64'),
    'aarch64-unknown-linux-gnu': ('linux', 'aarch64'),
    'x86_64-apple-darwin': ('darwin', 'x86_64'),
    'aarch64-apple-darwin': ('darwin', 'arm64'),
    'x86_64-pc-windows-msvc': ('win32', 'amd64'),
}
CHECKS = ('artifact-identity', 'host-identity', 'install', 'status', 'doctor',
          'merge', 'fault-separation', 'recovery')


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(chunk)
    return value.hexdigest()


def snapshot(root):
    entries = {}
    for path in sorted(root.rglob('*')):
        relative = path.relative_to(root).as_posix()
        info = path.lstat()
        if path.is_symlink():
            entries[relative] = ['link', os.readlink(path)]
        elif path.is_file():
            entries[relative] = ['file', info.st_mode & 0o777, digest(path)]
        else:
            entries[relative] = ['other', info.st_mode & 0o777]
    return entries


def envelope(result):
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise AssertionError(
            f'invalid JSON stdout: {error}: {result.stdout!r} {result.stderr!r}'
        ) from error


def run(binary, root, *args, expect=0, env=None):
    result = subprocess.run(
        [str(binary), *args, '--directory', str(root), '--json'],
        capture_output=True, text=True, env=env, timeout=300)
    assert result.returncode == expect, (
        args, result.returncode, result.stdout, result.stderr)
    value = envelope(result)
    assert value.get('schema') == 'agentfoundry.command-result/v1', value
    return value


def version(binary):
    result = subprocess.run(
        [str(binary), 'version', '--json'], capture_output=True, text=True,
        timeout=60)
    assert result.returncode == 0, (result.stdout, result.stderr)
    value = envelope(result)
    assert value.get('schema') == 'agentfoundry.command-result/v1', value
    return value['data']


def identity_data(data, artifact):
    assert re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)',
                        data.get('version', '')), data
    assert re.fullmatch(r'[0-9a-f]{64}', data.get('payload_digest', '')), data
    assert data.get('source'), data
    assert data.get('fault_injection') is False, data
    return {
        'version': data['version'],
        'source': data['source'],
        'payload_digest': data['payload_digest'],
        'distribution': data.get('distribution'),
        'fault_injection': data['fault_injection'],
        'sha256': digest(artifact),
        'size': artifact.stat().st_size,
    }


def check_host(platform_id):
    expected_os, expected_arch = PLATFORMS[platform_id]
    host_os = sys.platform
    host_arch = platform.machine().lower()
    aliases = {
        'x86_64': {'x86_64', 'amd64'},
        'amd64': {'x86_64', 'amd64'},
        'aarch64': {'aarch64', 'arm64'},
        'arm64': {'aarch64', 'arm64'},
    }
    assert host_os == expected_os, (host_os, expected_os)
    assert host_arch in aliases[expected_arch], (host_arch, expected_arch)
    return {
        'system': platform.system(),
        'release': platform.release(),
        'machine': platform.machine(),
        'python': platform.python_version(),
        'platform': sys.platform,
    }


def payload_entries():
    root = Path(__file__).resolve().parents[2]
    manifest = json.loads((root / 'payload/manifest.json').read_text(encoding='utf-8'))
    return root, {entry['target']: entry for entry in manifest['entries']}


def check_install(binary, artifact, base):
    source, entries = payload_entries()
    root = base / 'fresh'
    installed = run(binary, root, 'install')
    assert installed['mutation'] == 'applied', installed
    state = json.loads((root / '.agentfoundry/state/manifest.json').read_text())
    assert state['binary_digest'] == digest(artifact), state['binary_digest']
    assert state['version'] == version(artifact)['version']
    installed = {entry['target'] for entry in state['entries']}
    for target, entry in entries.items():
        if entry.get('addon') or entry.get('adapter'):
            continue
        assert target in installed, target
    for entry in state['entries']:
        assert entry['target'] in entries, entry['target']
        blob = root / '.agentfoundry/state/objects' / entry['sha256']
        assert digest(blob) == entry['sha256'], entry['target']
        expected = (source / entries[entry['target']]['source']).read_bytes()
        assert hashlib.sha256(expected).hexdigest() == entry['sha256'], entry['target']
        if entry['ownership'] == 'managed_file':
            assert (root / entry['target']).read_bytes() == blob.read_bytes()
            assert blob.read_bytes() == expected, entry['target']
    bin_target = root / ('scripts/bin/agentfoundry.exe' if sys.platform == 'win32'
                         else 'scripts/bin/agentfoundry')
    assert bin_target.read_bytes() == artifact.read_bytes(), 'installed binary differs'
    assert bin_target.stat().st_size == artifact.stat().st_size
    if sys.platform != 'win32':
        assert bin_target.stat().st_mode & 0o111, 'installed binary is not executable'
    before = snapshot(root)
    assert run(binary, root, 'install')['data']['no_op'], 'idempotent install not no-op'
    assert snapshot(root) == before, 'idempotent install mutated consumer'
    return root, len(state['entries'])


def check_status(binary, artifact, root):
    data = version(artifact)
    status = run(binary, root, 'status')['data']
    assert status['installed'] is True, status
    assert status['version'] == data['version'], status
    assert status['source'] == data['source'], status
    assert status['payload_digest'] == data['payload_digest'], status
    assert status['running_executable'] == {
        'version': data['version'], 'source': data['source'],
        'payload_digest': data['payload_digest']}, status['running_executable']
    assert status['binary_identity_valid'] is True, status
    assert status['binary_executable'] is True, status
    assert status['missing_paths'] == [], status
    assert status['local_modifications'] == [], status
    assert status['resolution_pending'] is False, status
    assert status['entrypoint_integrated'] is True, status
    # This legacy field is recorded, not trusted as host proof.
    return {'generation': status['generation'],
            'platform_verification_field': status.get('platform_verification')}


def check_doctor(binary, root, base):
    path = base / 'empty-path'
    path.mkdir()
    environment = os.environ.copy()
    environment['PATH'] = str(path)
    result = subprocess.run(
        [str(binary), 'doctor', '--directory', str(root), '--json'],
        capture_output=True, text=True, env=environment, timeout=300)
    assert result.returncode == 0, (result.stdout, result.stderr)
    data = envelope(result)['data']
    checks = {check['capability']: check for check in data['checks']}
    assert set(checks) == {'git', 'python'}, checks
    assert all(check['result'] == 'Unknown' and 'unavailable' in check['reason']
               for check in checks.values()), checks
    assert data['installed'] is True and data['missing_paths'] == [], data
    assert not list(root.rglob('__pycache__')), 'doctor wrote bytecode'
    return sorted(checks)


def check_merge(binary, base):
    root = base / 'brownfield'
    root.mkdir()
    private = b'private instructions\r\n'
    architecture = b'private application architecture\r\n'
    (root / 'AGENTS.md').write_bytes(private)
    (root / 'docs').mkdir()
    (root / 'docs/ARCHITECTURE.md').write_bytes(architecture)
    run(binary, root, 'install', '--merge', '--refresh-agent-shim', '--claude')
    agents = (root / 'AGENTS.md').read_bytes()
    assert agents.startswith(private), agents
    assert agents.count(b'<!-- AGENTFOUNDRY:BEGIN -->') == 1, agents
    assert agents.count(b'<!-- AGENTFOUNDRY:END -->') == 1, agents
    assert (root / 'docs/ARCHITECTURE.md').read_bytes() == architecture
    claude = (root / 'CLAUDE.md')
    assert claude.is_file() and claude.read_text().count('@AGENTS.md') == 1
    status = run(binary, root, 'status')['data']
    assert status['installed'] is True and status['entrypoint_integrated'] is True, status
    assert set(status['adapters']) == {'agents-md', 'claude'}, status['adapters']
    assert run(binary, root, 'doctor')['data']['missing_paths'] == []
    # Unmanaged collision is preserved and not adopted into installed ownership.
    collision = base / 'unmanaged'
    target = collision / 'docs/agentfoundry/WORKFLOW.md'
    target.parent.mkdir(parents=True)
    target.write_bytes(b'consumer protocol\r\n')
    (collision / 'docs/ARCHITECTURE.md').write_bytes(architecture)
    run(binary, collision, 'install', '--merge')
    state = json.loads((collision / '.agentfoundry/state/manifest.json').read_text())
    assert not any(entry['target'] == 'docs/agentfoundry/WORKFLOW.md'
                   for entry in state['entries'])
    assert target.read_bytes() == b'consumer protocol\r\n'
    assert (collision / 'docs/ARCHITECTURE.md').read_bytes() == architecture
    # Local edits survive an ordinary offline update/no-op.
    workflow = root / 'docs/agentfoundry/WORKFLOW.md'
    edited = workflow.read_bytes() + b'\nconsumer note\n'
    workflow.write_bytes(edited)
    before = snapshot(root)
    result = run(binary, root, 'update', '--offline')['data']
    assert result['no_op'] is True, result
    assert workflow.read_bytes() == edited
    assert snapshot(root) == before, 'offline update mutated merged consumer'
    return {'adapters': sorted(status['adapters']), 'local_edit_preserved': True,
            'unmanaged_collision_preserved': True}


def check_fault_separation(packaged, fault, packaged_identity, base):
    assert packaged.resolve() != fault.resolve(), 'packaged and fault binaries are identical'
    fault_identity = version(fault)
    assert packaged_identity['fault_injection'] is False
    assert fault_identity.get('fault_injection') is True, fault_identity
    assert digest(packaged) != digest(fault), 'packaged bytes contain fault build'
    root = base / 'no-bypass'
    environment = os.environ.copy()
    environment['AGENTFOUNDRY_TEST_CRASH'] = 'before-journal'
    result = subprocess.run(
        [str(packaged), 'install', '--directory', str(root), '--json'],
        capture_output=True, text=True, env=environment, timeout=300)
    assert result.returncode == 0, (result.returncode, result.stdout, result.stderr)
    assert envelope(result)['mutation'] == 'applied'
    assert (root / 'AGENTS.md').is_file()
    return {'fault_sha256': digest(fault),
            'fault_version': fault_identity.get('version'),
            'fault_distribution': fault_identity.get('distribution')}


def crash(binary, root, checkpoint):
    environment = os.environ.copy()
    environment['AGENTFOUNDRY_TEST_CRASH'] = checkpoint
    result = subprocess.run(
        [str(binary), 'install', '--directory', str(root), '--json'],
        capture_output=True, text=True, env=environment, timeout=300)
    assert result.returncode == 86, (checkpoint, result.returncode,
                                     result.stdout, result.stderr)


def check_recovery(packaged, fault, base):
    rows = {}
    empty = base / 'recover-before-journal'
    crash(fault, empty, 'before-journal')
    assert not (empty / 'AGENTS.md').exists(), 'before-journal crash mutated payload'
    outcome = run(packaged, empty, 'recover')
    assert outcome['data']['recovery'] == 'not_needed', outcome
    rows['before-journal'] = 'not_needed'

    partial = base / 'recover-partial'
    crash(fault, partial, 'applied:AGENTS.md')
    assert (partial / 'AGENTS.md').is_file(), 'partial crash did not apply AGENTS.md'
    journal = partial / '.agentfoundry/runtime/transaction.json'
    record = json.loads(journal.read_text())
    backup = partial / '.agentfoundry/backups' / record['id'] / 'plan.json'
    backup_bytes = backup.read_bytes()
    outcome = run(packaged, partial, 'recover')
    assert outcome['data']['recovery'] == 'rolled_back', outcome
    assert not (partial / 'AGENTS.md').exists(), 'rollback retained applied target'
    assert not journal.exists(), 'rollback retained journal'
    assert backup.read_bytes() == backup_bytes, 'rollback discarded evidence'
    rows['applied:AGENTS.md'] = 'rolled_back'

    committed = base / 'recover-committed'
    crash(fault, committed, 'core-committed-durable')
    with (committed / 'AGENTS.md').open('ab') as stream:
        stream.write(b'\npost-commit consumer edit\n')
    outcome = run(packaged, committed, 'recover')
    assert outcome['data']['recovery'] == 'committed', outcome
    assert (committed / 'AGENTS.md').read_bytes().endswith(
        b'post-commit consumer edit\n'), 'committed recovery replaced consumer edit'
    status = run(packaged, committed, 'status')['data']
    assert status['installed'] is True and status['binary_identity_valid'] is True, status
    installed = committed / ('scripts/bin/agentfoundry.exe' if sys.platform == 'win32'
                             else 'scripts/bin/agentfoundry')
    assert installed.read_bytes() == fault.read_bytes(), 'committed transaction binary differs'
    rows['core-committed-durable'] = 'committed'
    return rows


def load_source_freeze(path):
    data = json.loads(path.read_text(encoding='utf-8'))
    assert data.get('schema') == 'agentfoundry.source-freeze/v1', data.get('schema')
    assert re.fullmatch(r'[0-9a-f]{64}', data.get('digest', '')), data.get('digest')
    files = data.get('files', {})
    assert files and data.get('count') == len(files), (data.get('count'), len(files))
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', required=True, choices=sorted(PLATFORMS))
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--fault-binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--expected-distribution', default='development',
                        choices=('development', 'stable'))
    parser.add_argument('--source-freeze', type=Path)
    args = parser.parse_args()

    started = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    artifact = args.binary.resolve()
    fault = args.fault_binary.resolve()
    assert artifact.is_file(), artifact
    assert fault.is_file(), fault
    host = check_host(args.platform)
    data = version(artifact)
    assert data.get('distribution') == args.expected_distribution, data
    artifact_identity = identity_data(data, artifact)
    fault_identity = version(fault)
    source_freeze = load_source_freeze(args.source_freeze) if args.source_freeze else None

    rows = []
    skips = []
    results = {}
    current = None

    def execute(check, function):
        nonlocal current
        current = check
        begin = time.time()
        detail = function()
        results[check] = detail
        rows.append({'id': 'T-060.' + check, 'status': 'passed',
                     'elapsed_ms': int((time.time() - begin) * 1000),
                     'detail': detail})

    failure = None
    with tempfile.TemporaryDirectory(prefix='agentfoundry-native-matrix-') as directory:
        base = Path(directory)
        try:
            execute('artifact-identity', lambda: artifact_identity)
            execute('host-identity', lambda: host)
            root = None
            entry_count = None
            def installation():
                nonlocal root, entry_count
                root, entry_count = check_install(artifact, artifact, base)
                return {'entries': entry_count}
            execute('install', installation)
            execute('status', lambda: check_status(artifact, artifact, root))
            execute('doctor', lambda: check_doctor(artifact, root, base))
            execute('merge', lambda: check_merge(artifact, base))
            execute('fault-separation', lambda: check_fault_separation(
                artifact, fault, artifact_identity, base))
            execute('recovery', lambda: check_recovery(artifact, fault, base))
        except AssertionError as error:
            failure = str(error)
            rows.append({'id': 'T-060.' + (current or 'setup'), 'status': 'failed',
                         'error': failure})

    proof = {
        'schema': 'agentfoundry.native-matrix/v1',
        'scenario': 'T-060',
        'status': 'failed' if failure else 'passed',
        'error': failure,
        'platform': args.platform,
        'host': host,
        'artifact': artifact_identity,
        'fault_build': results.get('fault-separation'),
        'source_freeze': None if source_freeze is None else {
            'digest': source_freeze['digest'],
            'count': source_freeze['count'],
            'base_commit': source_freeze.get('base_commit'),
        },
        'started_at': started,
        'finished_at': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'rows': rows,
        'skips': skips,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(proof, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'status': proof['status'], 'platform': args.platform,
                      'artifact_sha256': artifact_identity['sha256'],
                      'rows': len(rows), 'output': str(args.output)}, sort_keys=True))
    if failure:
        sys.exit(1)


if __name__ == '__main__':
    main()
