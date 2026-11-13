#!/usr/bin/env python3
"""OS lock contention exercised through the packaged executable."""
import os
import json
if os.name == 'nt':
    import ctypes
    import msvcrt
    from ctypes import wintypes
else:
    import fcntl
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
        if os.name == 'nt':
            class Overlapped(ctypes.Structure):
                _fields_ = [('internal', ctypes.c_size_t), ('internal_high', ctypes.c_size_t),
                            ('offset', wintypes.DWORD), ('offset_high', wintypes.DWORD),
                            ('event', wintypes.HANDLE)]
            kernel = ctypes.WinDLL('kernel32', use_last_error=True)
            kernel.LockFileEx.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD,
                                          wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(Overlapped)]
            kernel.LockFileEx.restype = wintypes.BOOL
            overlap = Overlapped()
            assert kernel.LockFileEx(msvcrt.get_osfhandle(handle.fileno()), 3, 0, 0xffffffff,
                                     0xffffffff, ctypes.byref(overlap)), ctypes.get_last_error()
        else:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        result = subprocess.run([str(binary), 'install', '--directory', str(root), '--json'], capture_output=True, text=True)
        assert result.returncode == 9, result.stdout
        assert json.loads(result.stdout)['code'] == 'LOCK_BUSY'
        assert not (root / 'AGENTS.md').exists()
        assert not (root / '.agentfoundry/runtime/transaction.json').exists()
    result = subprocess.run([str(binary), 'install', '--directory', str(root), '--json'], capture_output=True, text=True)
    assert result.returncode == 0, result.stdout
    assert (root / 'AGENTS.md').is_file()
    # Both saved-plan attempts must fail while an independent process holds the lock.
    workflow = root / 'docs/agentfoundry/WORKFLOW.md'
    workflow.write_bytes(b'local generation edit\n')
    plan = root.parent / (root.name + '-plan.json')
    try:
        preview = subprocess.run([str(binary), 'install', '--override', '--dry-run', '--plan-out', str(plan),
                                  '--directory', str(root), '--json'], capture_output=True, text=True)
        assert preview.returncode == 0, preview.stdout
        before_files = {p.relative_to(root).as_posix(): p.read_bytes() for p in root.rglob('*') if p.is_file() and p != lock}
        with lock.open('r+b') as handle:
            if os.name == 'nt':
                overlap = Overlapped()
                assert kernel.LockFileEx(msvcrt.get_osfhandle(handle.fileno()), 3, 0, 0xffffffff,
                                         0xffffffff, ctypes.byref(overlap)), ctypes.get_last_error()
            else:
                fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            command = [str(binary), 'install', '--apply-plan', str(plan), '--directory', str(root), '--json']
            workers = [subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for _ in range(2)]
            for worker in workers:
                stdout, stderr = worker.communicate(timeout=60)
                response = json.loads(stdout)
                assert worker.returncode == 9 and response['code'] == 'LOCK_BUSY', (stdout, stderr)
                assert response['mutation'] == 'none'
            assert {p.relative_to(root).as_posix(): p.read_bytes() for p in root.rglob('*') if p.is_file() and p != lock} == before_files
        applied = subprocess.run(command, capture_output=True, text=True)
        assert applied.returncode == 0 and json.loads(applied.stdout)['mutation'] == 'applied', applied.stdout
        final_files = {p.relative_to(root).as_posix(): p.read_bytes() for p in root.rglob('*') if p.is_file()}
        stale = subprocess.run(command, capture_output=True, text=True)
        assert stale.returncode == 4 and json.loads(stale.stdout)['code'] == 'WORKSPACE_DRIFT', stale.stdout
        assert {p.relative_to(root).as_posix(): p.read_bytes() for p in root.rglob('*') if p.is_file()} == final_files
    finally:
        plan.unlink(missing_ok=True)
print('public install rejected held OS lock and succeeded after lock release')
