#!/usr/bin/env python3
"""Exercise a published staging update interrupted before and after core commit on Windows.

Pass an isolated fault-enabled baseline executable and expected published candidate SHA-256.
The baseline must already target the staging endpoint; never use a production installation.
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
assert sys.platform == 'win32' and len(expected) == 64
identity = json.loads(subprocess.check_output([str(baseline), '--version', '--json']))['data']
assert identity['fault_injection'] and identity['version'] == '0.1.0'

for checkpoint, outcome in [('journal-durable', 'rolled_back'), ('core-committed-durable', 'committed')]:
    with tempfile.TemporaryDirectory(prefix='af-published-crash-') as tmp:
        root = Path(tmp)
        subprocess.run([str(baseline), 'install', '--directory', str(root), '--json'], check=True, capture_output=True)
        launcher = root / 'scripts/agentfoundry.ps1'
        binary = root / 'scripts/bin/agentfoundry.exe'
        old_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
        def invoke(command, crash=None):
            env = os.environ.copy()
            env.pop('AGENTFOUNDRY_TEST_CRASH', None)
            if crash:
                env['AGENTFOUNDRY_TEST_CRASH'] = crash
            return subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', str(launcher), command, '--directory', str(root), '--json'], env=env, capture_output=True, text=True, timeout=120)
        interrupted = invoke('update', checkpoint)
        assert interrupted.returncode == 86, (checkpoint, interrupted.returncode, interrupted.stdout, interrupted.stderr)
        journal = root / '.agentfoundry/runtime/transaction.json'
        activation = root / '.agentfoundry/runtime/activation.json'
        assert journal.is_file() and activation.is_file()
        recovered = invoke('recover')
        assert recovered.returncode == 0, (recovered.stdout, recovered.stderr)
        assert json.loads(recovered.stdout)['data']['recovery'] == outcome
        assert not journal.exists() and activation.is_file()
        if outcome == 'rolled_back':
            assert hashlib.sha256(binary.read_bytes()).hexdigest() == old_hash
        completed = invoke('recover')
        assert completed.returncode == 0, (completed.stdout, completed.stderr)
        result = json.loads(completed.stdout)['data']['recovery']
        if outcome == 'rolled_back':
            assert result == 'activation_cancelled'
            updated = invoke('update')
            assert updated.returncode == 0, (updated.stdout, updated.stderr)
        else:
            assert result == 'activation_repaired'
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == expected
        assert not journal.exists() and not activation.exists()
        again = invoke('recover')
        assert again.returncode == 0 and json.loads(again.stdout)['data']['recovery'] == 'not_needed'
        print(f'{checkpoint}: {outcome}, candidate active, retry idempotent')
