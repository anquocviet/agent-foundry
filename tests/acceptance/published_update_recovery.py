#!/usr/bin/env python3
"""Windows process-exit recovery against a published candidate, in disposable consumers.

Usage: python -B published_update_recovery.py FAULT_BASELINE EXPECTED_SHA256 EXPECTED_VERSION
The isolated 0.1.0 fault build must pin the intended release repository. This is not
power-loss or ReplaceFileW partial-failure evidence; no installed workspace is used.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

baseline = Path(sys.argv[1]).resolve()
expected = sys.argv[2]
version = sys.argv[3]
assert sys.platform == 'win32'
assert len(expected) == 64 and all(c in '0123456789abcdef' for c in expected)
identity = json.loads(subprocess.check_output([str(baseline), '--version', '--json']))['data']
assert identity['fault_injection'] and identity['version'] == '0.1.0'


def snapshot(root):
    return {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in root.rglob('*') if p.is_file()}


scenarios = [
    ('prepared-durable', 'rolled_back', None, None),
    ('journal-durable', 'rolled_back', None, None),
    ('applied:.agentfoundry/state/manifest.json', 'committed', None, None),
    ('core-committed-durable', 'committed', None, None),
    ('before-complete', 'committed', None, None),
    ('journal-durable', 'rolled_back', 'rollback-durable', None),
    ('journal-durable', 'rolled_back', 'recovered-durable', None),
    ('core-committed-durable', 'committed', None, 'applied:scripts/bin/agentfoundry.exe'),
    ('core-committed-durable', 'committed', None, 'core-committed-durable'),
]
for checkpoint, outcome, rollback, repair in scenarios:
    with tempfile.TemporaryDirectory(prefix='af-published-crash-') as tmp:
        root = Path(tmp)
        subprocess.run([str(baseline), 'install', '--directory', str(root), '--json'],
                       check=True, capture_output=True, timeout=120)
        launcher = root / 'scripts/agentfoundry.ps1'
        binary = root / 'scripts/bin/agentfoundry.exe'
        old_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
        state = root / '.agentfoundry/state/manifest.json'
        old_state = state.read_bytes()
        agents = root / 'AGENTS.md'
        agents.write_bytes(b'Consumer prefix\n' + agents.read_bytes() + b'\nConsumer suffix\n')
        workflow = root / 'docs/agentfoundry/WORKFLOW.md'
        workflow.write_bytes(workflow.read_bytes() + b'\nConsumer local note\n')
        preserved = {p: p.read_bytes() for p in (agents, workflow)}

        def invoke(command, crash=None, dry=False):
            env = os.environ.copy()
            env.pop('AGENTFOUNDRY_TEST_CRASH', None)
            if crash:
                env['AGENTFOUNDRY_TEST_CRASH'] = crash
            result = subprocess.run(
                ['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                 str(launcher), command, '--directory', str(root), '--json',
                 *(['--dry-run'] if dry else [])],
                env=env, capture_output=True, text=True, timeout=180)
            assert result.returncode == (86 if crash else 0), (
                checkpoint, rollback, repair, result.returncode, result.stdout, result.stderr)
            if not crash:
                envelope = json.loads(result.stdout)
                assert envelope['code'] == 'OK', envelope
                return envelope['data']

        invoke('update', checkpoint)
        journal = root / '.agentfoundry/runtime/transaction.json'
        activation = root / '.agentfoundry/runtime/activation.json'
        assert journal.is_file() and activation.is_file()
        frozen = snapshot(root)
        invoke('recover', dry=True)
        assert snapshot(root) == frozen, 'recovery preview mutated consumer/evidence'
        if rollback:
            invoke('recover', rollback)
            assert journal.is_file() and activation.is_file()
        assert invoke('recover')['recovery'] == outcome
        assert not journal.exists() and activation.is_file()
        if outcome == 'rolled_back':
            assert hashlib.sha256(binary.read_bytes()).hexdigest() == old_hash
            assert state.read_bytes() == old_state
            assert invoke('recover')['recovery'] == 'activation_cancelled'
            invoke('update')
        else:
            if repair:
                invoke('recover', repair)
                assert journal.is_file() and activation.is_file()
                assert hashlib.sha256(binary.read_bytes()).hexdigest() == expected
                repaired = invoke('recover')['recovery']
                assert repaired == ('rolled_back' if repair.startswith('applied:') else 'committed')
                assert hashlib.sha256(binary.read_bytes()).hexdigest() == (
                    old_hash if repair.startswith('applied:') else expected)
            assert invoke('recover')['recovery'] == 'activation_repaired'
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == expected
        active = json.loads(subprocess.check_output([str(binary), '--version', '--json']))['data']
        assert active['version'] == version and active['distribution'] == 'stable', active
        assert not active['fault_injection'], active
        assert not journal.exists() and not activation.exists()
        for path, contents in preserved.items():
            assert path.read_bytes() == contents, path
        invoke('status')
        invoke('doctor')
        for _ in range(2):
            assert invoke('recover')['recovery'] == 'not_needed'
        print(f'{checkpoint} rollback={rollback} repair={repair}: {outcome}, '
              'candidate active, local edits preserved, recovery idempotent', flush=True)
