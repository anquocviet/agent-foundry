#!/usr/bin/env python3
"""Usage: python3 -B tests/acceptance/published_upgrade.py BASELINE EXPECTED_VERSION EXPECTED_SHA256

Use the production v0.1.0 binary and the approved newer release's native asset digest.
Creates only a disposable consumer; never substitutes release endpoints.
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

baseline = Path(sys.argv[1]).resolve()
version, digest = sys.argv[2:]
assert len(digest) == 64 and all(c in '0123456789abcdef' for c in digest)


def execute(argv):
    result = subprocess.run(argv, capture_output=True, text=True, timeout=180)
    assert result.returncode == 0, (result.returncode, result.stdout, result.stderr)
    envelope = json.loads(result.stdout)
    assert envelope['code'] == 'OK', envelope
    return envelope['data']


identity = execute([str(baseline), '--version', '--json'])
assert identity['distribution'] == 'stable' and not identity['fault_injection'], identity
assert identity['version'] == '0.1.0', identity
assert tuple(map(int, version.split('.'))) > (0, 1, 0), 'requires a newer production release'
with tempfile.TemporaryDirectory(prefix='af-production-upgrade-') as directory:
    root = Path(directory)
    execute([str(baseline), 'install', '--directory', directory, '--json'])
    binary = root / ('scripts/bin/agentfoundry.exe' if sys.platform == 'win32'
                     else 'scripts/bin/agentfoundry')
    launcher = root / 'scripts/agentfoundry.ps1'

    def invoke(command):
        prefix = (['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass',
                   '-File', str(launcher)] if sys.platform == 'win32' else [str(binary)])
        return execute([*prefix, command, '--directory', directory, '--json'])

    agents = root / 'AGENTS.md'
    agents.write_bytes(b'Consumer-owned prefix\n' + agents.read_bytes() + b'\nConsumer-owned suffix\n')
    workflow = root / 'docs/agentfoundry/WORKFLOW.md'
    edited = workflow.read_bytes() + b'\nConsumer local note\n'
    workflow.write_bytes(edited)
    invoke('update')
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
    active = execute([str(binary), '--version', '--json'])
    assert active['version'] == version and active['distribution'] == 'stable', active
    assert not active['fault_injection'], active
    assert workflow.read_bytes() == edited
    assert agents.read_bytes().startswith(b'Consumer-owned prefix\n')
    assert agents.read_bytes().endswith(b'\nConsumer-owned suffix\n')
    invoke('status')
    invoke('doctor')
    for _ in range(2):
        assert invoke('recover')['recovery'] == 'not_needed'
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest
    assert workflow.read_bytes() == edited
    print(f'production 0.1.0 -> {version}: local edits preserved, digest matched, recovery idempotent')
