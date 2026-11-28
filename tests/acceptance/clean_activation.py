#!/usr/bin/env python3
"""Exercise offline activation recovery after a clean-update process exit."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

check, old, candidate = (Path(arg).resolve() for arg in sys.argv[1:])
with tempfile.TemporaryDirectory(prefix='af-clean-activation-') as directory:
    root = Path(directory) / 'consumer'
    env = {**os.environ, 'AF_CHECK_CLEAN': '1', 'AGENTFOUNDRY_TEST_CRASH': 'core-committed-durable'}
    crashed = subprocess.run([str(check), str(old), str(candidate), str(root)], env=env, capture_output=True, text=True)
    assert crashed.returncode == 86, (crashed.returncode, crashed.stdout, crashed.stderr)
    assert not (root / '.agentfoundry/runtime/resolution.json').exists()
    record = root / '.agentfoundry/runtime/activation.json'
    assert record.is_file()
    def snapshot():
        return {p.relative_to(root): (hashlib.sha256(p.read_bytes()).hexdigest(), p.stat().st_mode)
                for p in root.rglob('*') if p.is_file()}
    def recover(*args):
        result = subprocess.run([str(old), 'recover', '--directory', str(root), '--json', *args], capture_output=True, text=True)
        assert result.returncode == 0, (result.returncode, result.stdout, result.stderr)
        return json.loads(result.stdout)['data']['recovery']
    before = snapshot()
    recover('--dry-run')
    assert snapshot() == before
    assert recover() == 'committed'
    assert record.is_file()
    binary = root / ('scripts/bin/agentfoundry.exe' if sys.platform == 'win32' else 'scripts/bin/agentfoundry')
    binary.unlink()
    before = snapshot()
    recover('--dry-run')
    assert snapshot() == before
    assert recover() == 'activation_repaired'
    assert hashlib.sha256(binary.read_bytes()).digest() == hashlib.sha256(candidate.read_bytes()).digest()
    assert not record.exists() and not (root / '.agentfoundry/runtime/resolution.json').exists()
    assert recover() == 'not_needed'
print('clean standalone offline activation recovery passed')
