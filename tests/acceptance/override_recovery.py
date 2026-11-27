#!/usr/bin/env python3
"""Cross-platform T-007 override crash rollback acceptance test.

Usage: python3 -B tests/acceptance/override_recovery.py BASELINE_BINARY FAULT_BINARY
"""
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def snapshot(root: Path, include_backups: bool = True) -> dict[str, tuple[str, str | None, int | bool | None]]:
    entries: dict[str, tuple[str, str | None, int | bool | None]] = {}
    if not root.exists():
        return entries
    for path in sorted(root.rglob('*')):
        rel = path.relative_to(root).as_posix()
        if not include_backups and (rel == '.agentfoundry/backups' or rel.startswith('.agentfoundry/backups/')):
            continue
        if path.is_symlink():
            entries[rel] = ('symlink', os.readlink(path), None)
        elif path.is_dir():
            entries[rel] = ('dir', None, None)
        elif path.is_file():
            st = path.lstat()
            mode = (st.st_mode & 0o777) if os.name == 'posix' else bool(st.st_mode & stat.S_IWRITE)
            entries[rel] = ('file', sha256_bytes(path.read_bytes()), mode)
    return entries


def backup_plans(root: Path) -> list[str]:
    backups_dir = root / '.agentfoundry/backups'
    if not backups_dir.exists():
        return []
    return sorted(p.relative_to(root).as_posix() for p in backups_dir.glob('*/plan.json'))


def clean_env(extra: dict[str, str] | None = None) -> dict[str, str]:
    env = os.environ.copy()
    env.pop('AGENTFOUNDRY_TEST_CRASH', None)
    env.pop('AGENTFOUNDRY_TEST_RECOVERY_ERROR', None)
    if extra:
        env.update(extra)
    return env


def run_json(cmd: list[str], expected_exit: int = 0, extra_env: dict[str, str] | None = None) -> dict | None:
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=180, env=clean_env(extra_env))
    assert result.returncode == expected_exit, (
        f'expected exit {expected_exit}, got {result.returncode}\ncmd: {cmd}\nstdout: {result.stdout}\nstderr: {result.stderr}'
    )
    if expected_exit == 86:
        return None
    envelope = json.loads(result.stdout)
    assert envelope['schema'] == 'agentfoundry.command-result/v1', envelope
    return envelope


def invoke_installed(
    root: Path, *args: str, expected_exit: int = 0, extra_env: dict[str, str] | None = None
) -> dict | None:
    if sys.platform == 'win32':
        cmd = [
            'powershell.exe',
            '-NoProfile',
            '-ExecutionPolicy',
            'Bypass',
            '-File',
            str(root / 'scripts/agentfoundry.ps1'),
            *args,
            '--directory',
            str(root),
            '--json',
        ]
    else:
        cmd = [
            str(root / 'scripts/bin/agentfoundry'),
            *args,
            '--directory',
            str(root),
            '--json',
        ]
    return run_json(cmd, expected_exit=expected_exit, extra_env=extra_env)


def main() -> None:
    if len(sys.argv) < 3:
        raise SystemExit('Usage: override_recovery.py BASELINE_BINARY FAULT_BINARY')
    baseline = Path(sys.argv[1]).resolve()
    fault = Path(sys.argv[2]).resolve()
    assert baseline.is_file(), f'missing baseline binary: {baseline}'
    assert fault.is_file(), f'missing fault binary: {fault}'

    base_ver = run_json([str(baseline), '--version', '--json'])
    assert base_ver is not None and base_ver['code'] == 'OK', base_ver
    assert base_ver['data']['fault_injection'] is False, base_ver
    baseline_version = base_ver['data']['version']

    fault_ver = run_json([str(fault), '--version', '--json'])
    assert fault_ver is not None and fault_ver['code'] == 'OK', fault_ver
    assert fault_ver['data']['fault_injection'] is True, fault_ver

    assert baseline.read_bytes() != fault.read_bytes(), (
        'baseline and fault binaries must differ so binary replacement triggers checkpoint'
    )

    bin_rel = 'scripts/bin/agentfoundry.exe' if sys.platform == 'win32' else 'scripts/bin/agentfoundry'
    checkpoints = [
        'journal-durable',
        'applied:docs/agentfoundry/WORKFLOW.md',
        'applied:docs/agentfoundry/README.md',
        'applied:CLAUDE.md',
        f'applied:{bin_rel}',
    ]

    for checkpoint in checkpoints:
        with tempfile.TemporaryDirectory(prefix='af-override-recovery-') as tmp:
            tmp_path = Path(tmp).resolve()
            root = tmp_path / 'consumer space'
            sentinel = tmp_path / 'outside'
            sentinel.write_bytes(b'outside owner')

            installed = run_json([str(baseline), 'install', '--directory', str(root), '--json'])
            assert installed is not None and installed['code'] == 'OK', installed
            assert installed['mutation'] == 'applied', installed
            assert (root / bin_rel).is_file()
            if sys.platform == 'win32':
                assert (root / 'scripts/agentfoundry.ps1').is_file()

            workflow = root / 'docs/agentfoundry/WORKFLOW.md'
            workflow.write_bytes(workflow.read_bytes() + b'\nConsumer local workflow edit\n')

            agents = root / 'AGENTS.md'
            agents.write_bytes(b'Consumer-owned prefix\n' + agents.read_bytes() + b'\nConsumer-owned suffix\n')

            gitignore = root / '.gitignore'
            gitignore.write_bytes(gitignore.read_bytes() + b'\ncustom-ignored/\n')

            private_doc = root / 'docs/private-notes.md'
            private_doc.write_bytes(b'Consumer private documentation\n')

            readme = root / 'docs/agentfoundry/README.md'
            readme.unlink()

            claude = root / 'CLAUDE.md'
            assert not claude.exists()

            before_no_backups = snapshot(root, include_backups=False)
            plans_before = backup_plans(root)

            run_json(
                [str(fault), 'install', '--directory', str(root), '--override', '--claude', '--json'],
                expected_exit=86,
                extra_env={'AGENTFOUNDRY_TEST_CRASH': checkpoint},
            )

            journal = root / '.agentfoundry/runtime/transaction.json'
            assert journal.is_file(), (checkpoint, 'missing transaction journal')
            journal_data = json.loads(journal.read_bytes())
            assert journal_data['phase'] == 'Applying', (checkpoint, journal_data)
            tx_id = journal_data['id']
            backup_plan = root / '.agentfoundry/backups' / tx_id / 'plan.json'
            assert backup_plan.is_file(), (checkpoint, backup_plan)
            assert sentinel.read_bytes() == b'outside owner'

            frozen_after_crash = snapshot(root, include_backups=True)
            preview = invoke_installed(root, 'recover', '--dry-run')
            assert preview is not None and preview['code'] == 'OK', (checkpoint, preview)
            assert preview['data']['recovery'] == 'rollback', (checkpoint, preview)
            assert preview['mutation'] == 'none', (checkpoint, preview)
            assert snapshot(root, include_backups=True) == frozen_after_crash, (
                checkpoint,
                'recovery preview mutated consumer or evidence',
            )

            recovered = invoke_installed(root, 'recover')
            assert recovered is not None and recovered['code'] == 'OK', (checkpoint, recovered)
            assert recovered['data']['recovery'] == 'rolled_back', (checkpoint, recovered)
            assert recovered['mutation'] == 'rolled_back', (checkpoint, recovered)
            assert not journal.exists(), (checkpoint, 'journal remained after rollback')
            assert backup_plan.is_file(), (checkpoint, 'backup plan removed during rollback')
            assert len(backup_plans(root)) == len(plans_before) + 1
            assert snapshot(root, include_backups=False) == before_no_backups, (
                checkpoint,
                'consumer snapshot mismatch after override rollback',
            )
            assert sentinel.read_bytes() == b'outside owner'

            repeat = invoke_installed(root, 'recover')
            assert repeat is not None and repeat['code'] == 'OK', (checkpoint, repeat)
            assert repeat['data']['recovery'] == 'not_needed', (checkpoint, repeat)
            assert repeat['mutation'] == 'none', (checkpoint, repeat)
            assert backup_plan.is_file()

            final = run_json([str(baseline), 'install', '--directory', str(root), '--override', '--claude', '--json'])
            assert final is not None and final['code'] == 'OK', (checkpoint, final)
            assert final['mutation'] == 'applied', (checkpoint, final)
            assert readme.is_file()
            assert claude.is_file()
            assert private_doc.read_bytes() == b'Consumer private documentation\n'
            assert agents.read_bytes().startswith(b'Consumer-owned prefix\n')
            assert agents.read_bytes().endswith(b'\nConsumer-owned suffix\n')
            assert b'custom-ignored/' in gitignore.read_bytes()
            assert b'Consumer local workflow edit' not in workflow.read_bytes()

            offline_update = invoke_installed(root, 'update', '--offline')
            assert offline_update is not None and offline_update['code'] == 'OK', (checkpoint, offline_update)
            status = invoke_installed(root, 'status')
            assert status is not None and status['code'] == 'OK', (checkpoint, status)
            doctor = invoke_installed(root, 'doctor')
            assert doctor is not None and doctor['code'] == 'OK', (checkpoint, doctor)

            reinstall = run_json(
                [str(baseline), 'install', '--directory', str(root), '--override', '--claude', '--json']
            )
            assert reinstall is not None and reinstall['code'] == 'OK', (checkpoint, reinstall)
            assert reinstall['data']['no_op'] is True, (checkpoint, reinstall)
            assert reinstall['mutation'] == 'none', (checkpoint, reinstall)

            final_recover = invoke_installed(root, 'recover')
            assert final_recover is not None and final_recover['code'] == 'OK', (checkpoint, final_recover)
            assert final_recover['data']['recovery'] == 'not_needed', (checkpoint, final_recover)
            assert sentinel.read_bytes() == b'outside owner'

    print(
        f'override crash rollback passed ({len(checkpoints)} precommit checkpoints, '
        f'baseline={baseline_version}, platform={sys.platform})'
    )


if __name__ == '__main__':
    main()
