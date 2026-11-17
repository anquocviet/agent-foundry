#!/usr/bin/env python3
"""Native Windows ReplaceFileW probe and modeled partial-outcome recovery.

Usage: python -B tests/acceptance/windows_replacefile.py FAULT_BINARY
1176/1177 layouts follow Microsoft's ReplaceFileW return-value contract:
https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew
These layouts are constructed, not claims that Windows returned those errors.
Production uses Rust fs::rename, not ReplaceFileW. Never run on a real consumer.
"""
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from windows_launcher import digest, stage_activation

assert sys.platform == 'win32'
baseline = Path(sys.argv[1]).resolve()
identity = json.loads(subprocess.check_output([str(baseline), 'version', '--json']))['data']
assert identity['fault_injection'] and identity['source'].startswith('development-sha256:')
kernel = ctypes.WinDLL('kernel32', use_last_error=True)
replace = kernel.ReplaceFileW
replace.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.LPCWSTR,
                    wintypes.DWORD, wintypes.LPVOID, wintypes.LPVOID]
replace.restype = wintypes.BOOL
create = kernel.CreateFileW
create.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID,
                   wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
create.restype = wintypes.HANDLE
close = kernel.CloseHandle
close.argtypes = [wintypes.HANDLE]
close.restype = wintypes.BOOL


def snapshot(root):
    return {str(p.relative_to(root)): digest(p.read_bytes())
            for p in root.rglob('*') if p.is_file()}


with tempfile.TemporaryDirectory(prefix='af-replacefile-') as directory:
    base = Path(directory)
    target, replacement, backup = (base / name for name in ('target.exe', 'replacement.exe', 'backup.exe'))
    target.write_bytes(b'old')
    replacement.write_bytes(b'new')
    handle = create(str(target), 0x80000000, 1, None, 3, 0, None)
    assert handle != ctypes.c_void_p(-1).value, ctypes.get_last_error()
    try:
        assert not replace(str(target), str(replacement), str(backup), 0, None, None)
        error = ctypes.get_last_error()
        assert error in (32, 1175), error
        assert target.read_bytes() == b'old' and replacement.read_bytes() == b'new'
        assert not backup.exists()
        print(f'actual ReplaceFileW sharing refusal: WinError {error}, names/bytes intact', flush=True)
    finally:
        assert close(handle)
    assert replace(str(target), str(replacement), str(backup), 0, None, None), ctypes.get_last_error()
    assert target.read_bytes() == b'new' and backup.read_bytes() == b'old'
    assert not replacement.exists()
    print('actual ReplaceFileW success: replacement installed, original backup retained', flush=True)

    trusted = base / 'trusted'
    subprocess.run([str(baseline), 'install', '--directory', str(trusted), '--json'],
                   check=True, capture_output=True, timeout=120)
    launcher = trusted / 'scripts/agentfoundry.ps1'
    for index, layout in enumerate(('1175', '1176-backup', '1176-no-backup',
                                    '1177-backup', '1177-no-backup', 'pending-journal-missing')):
        root = base / layout
        subprocess.run([str(baseline), 'install', '--directory', str(root), '--json'],
                       check=True, capture_output=True, timeout=120)
        selected = root / 'scripts/bin/agentfoundry.exe'
        old = selected.read_bytes()
        candidate = old + b'\x00AF_PARTIAL_REPLACEMENT_CANDIDATE'
        record, retained, expected = stage_activation(root, f'fa{index:02x}', candidate, old)
        manifest = root / '.agentfoundry/state/manifest.json'
        state = manifest.read_bytes()
        agents = root / 'AGENTS.md'
        agents.write_bytes(b'Consumer prefix\n' + agents.read_bytes())
        consumer_bytes = agents.read_bytes()
        journal = root / '.agentfoundry/runtime/transaction.json'

        def invoke(dry=False, crash=None, code=0):
            env = os.environ.copy()
            env.pop('AGENTFOUNDRY_TEST_CRASH', None)
            if crash:
                env['AGENTFOUNDRY_TEST_CRASH'] = crash
            result = subprocess.run(
                ['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                 str(launcher), 'recover', '--directory', str(root), '--json',
                 *(['--dry-run'] if dry else [])],
                env=env, capture_output=True, text=True, timeout=120)
            assert result.returncode == code, (layout, result.returncode, result.stdout, result.stderr)
            return json.loads(result.stdout) if code != 86 else None

        if layout == 'pending-journal-missing':
            invoke(crash='applied:scripts/bin/agentfoundry.exe', code=86)
            assert journal.exists() and selected.read_bytes() == candidate
        original = selected.with_name('original-backup.exe')
        temp = selected.with_name('replacement.tmp')
        temp.write_bytes(candidate)
        if layout in ('1177-backup', '1177-no-backup'):
            selected.rename(original)
        elif layout in ('1176-no-backup', 'pending-journal-missing'):
            selected.unlink()
        elif layout == '1176-backup':
            assert selected.read_bytes() == old
        evidence = snapshot(root)
        if layout == 'pending-journal-missing':
            for dry in (True, False):
                refused = invoke(dry=dry, code=8)
                assert refused['code'] == 'RECOVERY_BLOCKED' and refused['mutation'] == 'none', refused
                assert snapshot(root) == evidence
            # Unknown absence is not authorized by the journal. Restore a known image
            # explicitly; never delete the journal or infer trust from a renamed file.
            selected.write_bytes(old)
            assert invoke()['data']['recovery'] == 'rolled_back'
        else:
            assert invoke(dry=True)['data']['recovery'] == 'activation_preview'
            assert snapshot(root) == evidence
        assert record.exists() and retained.read_bytes() == candidate
        assert invoke()['data']['recovery'] == 'activation_repaired'
        assert selected.read_bytes() == candidate and digest(selected.read_bytes()) == expected
        assert manifest.read_bytes() == state and agents.read_bytes() == consumer_bytes
        assert not journal.exists() and not record.exists()
        assert retained.read_bytes() == candidate and temp.read_bytes() == candidate
        if original.exists():
            assert original.read_bytes() == old
        for _ in range(2):
            assert invoke()['data']['recovery'] == 'not_needed'
        print(f'modeled {layout}: evidence retained, exact candidate restored, recovery idempotent', flush=True)
