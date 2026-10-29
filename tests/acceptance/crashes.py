#!/usr/bin/env python3
"""Requires explicit fault-injection feature binary, never default release binary."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
checkpoints = ['before-journal', 'prepared-durable', 'journal-durable', 'applied:AGENTS.md',
               'applied:.agentfoundry/state/manifest.json', 'core-committed-durable', 'before-complete']
if '--all-files' in sys.argv[2:]:
    with tempfile.TemporaryDirectory(prefix='agentfoundry-checkpoints-') as directory:
        preview = subprocess.run([str(binary), 'install', '--directory', str(Path(directory) / 'consumer'), '--dry-run', '--json'], capture_output=True, text=True, check=True)
        targets = [row['target'] for row in json.loads(preview.stdout)['data']['changes']]
    checkpoints = ['before-journal', 'prepared-durable', 'journal-durable', *('applied:' + target for target in targets), 'core-committed-durable', 'before-complete']
for checkpoint in checkpoints:
    with tempfile.TemporaryDirectory(prefix='agentfoundry-crash-') as directory:
        root = Path(directory) / 'consumer'
        sentinel = Path(directory) / 'outside'
        sentinel.write_bytes(b'outside owner')
        environment = os.environ.copy()
        environment['AGENTFOUNDRY_TEST_CRASH'] = checkpoint
        result = subprocess.run([str(binary), 'install', '--directory', str(root), '--json'], env=environment, capture_output=True, text=True)
        assert result.returncode == 86, (checkpoint, result.returncode, result.stdout, result.stderr)
        if checkpoint == 'before-journal':
            assert not (root / 'AGENTS.md').exists()
        elif checkpoint in ('applied:.agentfoundry/state/manifest.json', 'core-committed-durable', 'before-complete'):
            # Local edits after core commit are valid and must not be rolled back.
            with (root / 'AGENTS.md').open('ab') as file:
                file.write(b'\npost-commit consumer edit\n')
        if checkpoint == 'journal-durable' and os.name == 'posix':
            import fcntl
            lock_path = root / '.agentfoundry/runtime/operation.lock'
            with lock_path.open('rb') as lock:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                journal = root / '.agentfoundry/runtime/transaction.json'
                original_journal = journal.read_bytes()
                busy = subprocess.run([str(binary), 'recover', '--directory', str(root), '--json'], capture_output=True, text=True)
                assert busy.returncode == 9, (busy.stdout, busy.stderr)
                assert journal.read_bytes() == original_journal
                assert not (root / 'AGENTS.md').exists()
        if checkpoint == 'journal-durable':
            journal = root / '.agentfoundry/runtime/transaction.json'
            journal_bytes = journal.read_bytes()
            status = subprocess.run([str(binary), 'status', '--directory', str(root), '--json'], capture_output=True, text=True)
            assert status.returncode == 0 and json.loads(status.stdout)['data']['transaction_pending'] is True, (status.stdout, status.stderr)
            assert journal.read_bytes() == journal_bytes
            transaction = json.loads(journal_bytes)
            backup = root / '.agentfoundry/backups' / transaction['id'] / 'plan.json'
            original = backup.read_bytes()
            for corrupt in (None, b'{}'):
                if corrupt is None:
                    backup.unlink()
                else:
                    backup.write_bytes(corrupt)
                blocked = subprocess.run([str(binary), 'recover', '--directory', str(root), '--json'], capture_output=True, text=True)
                assert blocked.returncode == 8, (blocked.stdout, blocked.stderr)
                assert journal.read_bytes() == journal_bytes
                assert not (root / 'AGENTS.md').exists()
                backup.write_bytes(original)
            missing_preimage = json.loads(journal_bytes)
            missing_preimage['plan']['operations'][0].pop('before')
            corrupt = json.dumps(missing_preimage).encode()
            journal.write_bytes(corrupt)
            blocked = subprocess.run([str(binary), 'recover', '--directory', str(root), '--json'], capture_output=True, text=True)
            assert blocked.returncode == 8 and journal.read_bytes() == corrupt
            assert backup.read_bytes() == original and not (root / 'AGENTS.md').exists()
            for corrupt in (b'{', json.dumps({**transaction, 'schema': 'agentfoundry.journal/v999'}).encode(),
                            json.dumps({**transaction, 'phase': 'UnknownPhase'}).encode()):
                journal.write_bytes(corrupt)
                blocked = subprocess.run([str(binary), 'recover', '--directory', str(root), '--json'], capture_output=True, text=True)
                assert blocked.returncode == 8, (blocked.stdout, blocked.stderr)
                assert journal.read_bytes() == corrupt
                assert backup.read_bytes() == original
                assert not (root / 'AGENTS.md').exists()
            journal.write_bytes(journal_bytes)
        if checkpoint == 'applied:AGENTS.md':
            if os.name == 'posix':
                target = root / 'AGENTS.md'
                original_mode = target.stat().st_mode & 0o777
                target.chmod(original_mode ^ 0o100)
                blocked = subprocess.run([str(binary), 'recover', '--directory', str(root), '--json'], capture_output=True, text=True)
                assert blocked.returncode == 8, (blocked.stdout, blocked.stderr)
                assert target.stat().st_mode & 0o777 == original_mode ^ 0o100
                assert (root / '.agentfoundry/runtime/transaction.json').is_file()
                preview = subprocess.run([str(binary), 'recover', '--dry-run', '--directory', str(root), '--json'], capture_output=True, text=True)
                assert preview.returncode == 8, (preview.stdout, preview.stderr)
                target.chmod(original_mode)
                if os.geteuid() != 0:
                    root_mode = root.stat().st_mode & 0o777
                    journal = root / '.agentfoundry/runtime/transaction.json'
                    journal_data = json.loads(journal.read_bytes())
                    backup = root / '.agentfoundry/backups' / journal_data['id'] / 'plan.json'
                    backup_bytes = backup.read_bytes()
                    target_bytes = target.read_bytes()
                    root.chmod(0o555)
                    try:
                        denied = subprocess.run([str(binary), 'recover', '--directory', str(root), '--json'], capture_output=True, text=True)
                        assert denied.returncode == 1, (denied.stdout, denied.stderr)
                        assert json.loads(denied.stdout)['mutation'] == 'partial_recovery_required'
                        assert target.read_bytes() == target_bytes
                        assert json.loads(journal.read_bytes())['phase'] == 'RollingBack'
                        assert backup.read_bytes() == backup_bytes
                    finally:
                        root.chmod(root_mode)
            failure_environment = os.environ.copy()
            failure_environment['AGENTFOUNDRY_TEST_RECOVERY_ERROR'] = 'AGENTS.md'
            failed = subprocess.run([str(binary), 'recover', '--directory', str(root), '--json'], env=failure_environment, capture_output=True, text=True)
            assert failed.returncode == 1 and json.loads(failed.stdout)['mutation'] == 'partial_recovery_required', (failed.stdout, failed.stderr)
            assert not (root / 'AGENTS.md').exists()
            journal_data = json.loads((root / '.agentfoundry/runtime/transaction.json').read_bytes())
            operation = next(op for op in journal_data['plan']['operations'] if op['target'] == 'AGENTS.md')
            image = root / '.agentfoundry/backups' / journal_data['id'] / 'objects' / operation['after']
            (root / 'AGENTS.md').write_bytes(image.read_bytes())
            (root / 'AGENTS.md').chmod(operation['mode'])
            environment['AGENTFOUNDRY_TEST_CRASH'] = 'rollback:AGENTS.md'
            interrupted = subprocess.run([str(binary), 'recover', '--directory', str(root), '--json'], env=environment, capture_output=True, text=True)
            assert interrupted.returncode == 86, (interrupted.stdout, interrupted.stderr)
            assert not (root / 'AGENTS.md').exists()
        if checkpoint == 'prepared-durable':
            environment['AGENTFOUNDRY_TEST_CRASH'] = 'rollback-durable'
            interrupted = subprocess.run([str(binary), 'recover', '--directory', str(root), '--json'], env=environment, capture_output=True, text=True)
            assert interrupted.returncode == 86, (interrupted.stdout, interrupted.stderr)
            journal = root / '.agentfoundry/runtime/transaction.json'
            assert json.loads(journal.read_bytes())['phase'] == 'RollingBack'
            assert not (root / 'AGENTS.md').exists()
            environment['AGENTFOUNDRY_TEST_CRASH'] = 'recovered-durable'
            interrupted = subprocess.run([str(binary), 'recover', '--directory', str(root), '--json'], env=environment, capture_output=True, text=True)
            assert interrupted.returncode == 86, (interrupted.stdout, interrupted.stderr)
            assert json.loads(journal.read_bytes())['phase'] == 'Recovered'
            assert not (root / 'AGENTS.md').exists()
        journal_path = root / '.agentfoundry/runtime/transaction.json'
        transaction = json.loads(journal_path.read_bytes()) if journal_path.exists() else None
        result = subprocess.run([str(binary), 'recover', '--directory', str(root), '--json'], capture_output=True, text=True)
        assert result.returncode == 0, (checkpoint, result.stdout, result.stderr)
        recovery = json.loads(result.stdout)['data']['recovery']
        assert recovery == ('not_needed' if checkpoint == 'before-journal' else
                            'committed' if checkpoint in ('applied:.agentfoundry/state/manifest.json', 'core-committed-durable', 'before-complete') else 'rolled_back')
        if recovery == 'committed':
            assert (root / 'AGENTS.md').read_bytes().endswith(b'post-commit consumer edit\n')
        else:
            assert not (root / 'AGENTS.md').exists()
        if transaction is not None:
            image_kind = 'after' if recovery == 'committed' else 'before'
            objects = root / '.agentfoundry/backups' / transaction['id'] / 'objects'
            for operation in transaction['plan']['operations']:
                target = root / operation['target']
                digest = operation[image_kind]
                if digest is None:
                    assert not target.exists(), (checkpoint, operation['target'])
                elif recovery != 'committed' or operation['target'] != 'AGENTS.md':
                    assert target.read_bytes() == (objects / digest).read_bytes(), (checkpoint, operation['target'])
                    if os.name == 'posix':
                        assert target.stat().st_mode & 0o777 == operation['mode'], (checkpoint, operation['target'])
        again = subprocess.run([str(binary), 'recover', '--directory', str(root), '--json'], capture_output=True, text=True)
        assert again.returncode == 0 and json.loads(again.stdout)['data']['recovery'] == 'not_needed'
        assert sentinel.read_bytes() == b'outside owner'
print(f'public crash/recover: {len(checkpoints)} process checkpoints passed, with rollback, commit, evidence retention and outside sentinel checks')
