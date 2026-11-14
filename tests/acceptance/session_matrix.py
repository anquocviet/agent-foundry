#!/usr/bin/env python3
"""Run against check-session, old/target release binaries and a fault-injection binary."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

check, old, candidate, fault = (str(Path(arg).resolve()) for arg in sys.argv[1:])
with tempfile.TemporaryDirectory(prefix='af-session-matrix-') as directory:
    counter = 0
    def run(block, crash, **extra):
        global counter
        counter += 1
        env = os.environ.copy()
        env.update(AF_CHECK_CRASH=crash, AF_CHECK_CRASH_BINARY=fault, **extra)
        if block:
            env['AF_CHECK_BLOCK'] = '1'
        print(f'{"block" if block else "file"}: {crash} {extra}', flush=True)
        result = subprocess.run([check, old, candidate, str(Path(directory) / str(counter))],
                                env=env, text=True, capture_output=True)
        if result.returncode:
            raise RuntimeError(f'{result.returncode}: {result.stdout}\n{result.stderr}')
        return result.stdout

    for block in (False, True):
        listing = run(block, 'journal-durable', AF_CHECK_LIST='1')
        targets = [line.removeprefix('TARGET ') for line in listing.splitlines() if line.startswith('TARGET ')]
        assert targets and targets[-1] == '.agentfoundry/state/manifest.json', targets
        for checkpoint in ('before-journal', 'prepared-durable', 'journal-durable',
                           *(f'applied:{target}' for target in targets),
                           'core-committed-durable', 'before-complete'):
            run(block, checkpoint)
        for target in targets[:-1]:
            run(block, f'applied:{target}', AF_CHECK_ROLLBACK=f'rollback:{target}')
        for checkpoint in ('rollback-durable', 'recovered-durable'):
            run(block, f'applied:{targets[-2]}', AF_CHECK_ROLLBACK=checkpoint)
        repair_applied = 'applied:scripts/bin/agentfoundry.exe' if sys.platform == 'win32' else 'applied:scripts/bin/agentfoundry'
        for checkpoint in (repair_applied, 'core-committed-durable'):
            run(block, 'core-committed-durable', AF_CHECK_REPAIR_CRASH=checkpoint)
print('session matrix passed')
