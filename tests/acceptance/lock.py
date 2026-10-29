#!/usr/bin/env python3
"""POSIX cooperative lock contention exercised through the packaged executable."""
import fcntl
import json
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='agentfoundry-lock-') as directory:
    root = Path(directory)
    lock = root / '.agentfoundry/runtime/operation.lock'
    lock.parent.mkdir(parents=True)
    with lock.open('w+b') as handle:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        result = subprocess.run([str(binary), 'install', '--directory', str(root), '--json'], capture_output=True, text=True)
        assert result.returncode == 9, result.stdout
        assert json.loads(result.stdout)['code'] == 'LOCK_BUSY'
        assert not (root / 'AGENTS.md').exists()
        assert not (root / '.agentfoundry/runtime/transaction.json').exists()
    result = subprocess.run([str(binary), 'install', '--directory', str(root), '--json'], capture_output=True, text=True)
    assert result.returncode == 0, result.stdout
    assert (root / 'AGENTS.md').is_file()
print('public install rejected held OS lock and succeeded after lock release')
