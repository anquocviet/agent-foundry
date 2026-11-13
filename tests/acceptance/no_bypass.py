#!/usr/bin/env python3
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
identity = json.loads(subprocess.check_output([str(binary), '--version', '--json']))['data']
assert identity['fault_injection'] is False
with tempfile.TemporaryDirectory(prefix='agentfoundry-no-bypass-') as directory:
    environment = os.environ.copy()
    environment.update({'AGENTFOUNDRY_TEST_CRASH': 'journal-durable',
                        'AGENTFOUNDRY_RELEASE_ROOT': 'http://127.0.0.1:1/untrusted',
                        'AGENTFOUNDRY_SKIP_SELF_REPLACE': '1'})
    result = subprocess.run([str(binary), 'install', '--directory', directory, '--json'], env=environment, capture_output=True, text=True)
    assert result.returncode == 0, (result.stdout, result.stderr)
    assert json.loads(result.stdout)['mutation'] == 'applied'
    assert not (Path(directory) / '.agentfoundry/runtime/transaction.json').exists()
    # Exercise recovery too: an install-only probe cannot reach recovery error hooks.
    root = Path(directory).resolve()
    state_path = root / '.agentfoundry/state/manifest.json'
    state = json.loads(state_path.read_bytes())
    target = root / 'AGENTS.md'
    after = target.read_bytes()
    before = after + b'\nprior consumer image\n'
    import hashlib
    images = root / '.agentfoundry/backups/abc/objects'
    images.mkdir(parents=True)
    for image in (before, after):
        (images / hashlib.sha256(image).hexdigest()).write_bytes(image)
    root_str = f'\\\\?\\{root.resolve()}' if sys.platform == 'win32' else str(root)
    root_id = root_str if sys.platform == 'win32' else f'{root.stat().st_dev}:{root.stat().st_ino}:{root}'
    plan = {'schema': 'agentfoundry.stored-plan/v1', 'root': root_str,
            'root_identity': root_id,
            'payload_digest': state['payload_digest'],
            'choices': {key: False for key in ('merge', 'replace', 'refresh', 'wisdom', 'claude')},
            'operations': [{'target': 'AGENTS.md', 'before': hashlib.sha256(before).hexdigest(),
                            'after': hashlib.sha256(after).hexdigest(), 'mode': target.stat().st_mode & 0o777}]}
    compact = json.dumps(plan, separators=(',', ':')).encode()
    (images.parent / 'plan.json').write_bytes(compact)
    journal = root / '.agentfoundry/runtime/transaction.json'
    journal.write_text(json.dumps({'schema': 'agentfoundry.journal/v1', 'id': 'abc', 'root': root_str,
                                  'plan_hash': hashlib.sha256(compact).hexdigest(), 'phase': 'Applying', 'plan': plan}))
    environment['AGENTFOUNDRY_TEST_CRASH'] = 'rollback:AGENTS.md'
    environment['AGENTFOUNDRY_TEST_RECOVERY_ERROR'] = 'AGENTS.md'
    result = subprocess.run([str(binary), 'recover', '--directory', directory, '--json'], env=environment, capture_output=True, text=True)
    assert result.returncode == 0, (result.stdout, result.stderr)
    assert json.loads(result.stdout)['mutation'] == 'rolled_back'
    assert target.read_bytes() == before and not journal.exists()
    journal_bytes = json.dumps({'schema': 'agentfoundry.journal/v1', 'id': 'abc', 'root': root_str,
                                'plan_hash': hashlib.sha256(compact).hexdigest(), 'phase': 'Applying', 'plan': plan}).encode()
    for command in (['install', '--apply-plan', str(root.parent / 'missing-plan.json')],
                    ['update', '--offline', '--apply-plan', str(root.parent / 'missing-plan.json')],
                    ['migrate', 'harness', '--apply']):
        target.write_bytes(after)
        journal.write_bytes(journal_bytes)
        result = subprocess.run([str(binary), *command, '--directory', directory, '--json'], capture_output=True, text=True)
        assert result.returncode != 0, (command, result.stdout, result.stderr)
        assert json.loads(result.stdout)['mutation'] == 'rolled_back', (command, result.stdout)
        assert target.read_bytes() == before and not journal.exists()
    target.write_bytes(after)
    journal.write_bytes(journal_bytes)
    result = subprocess.run([str(binary), 'update', '--continue', '--directory', directory, '--json'],
                            capture_output=True, text=True)
    assert result.returncode != 0, (result.stdout, result.stderr)
    assert json.loads(result.stdout)['mutation'] == 'rolled_back', result.stdout
    assert target.read_bytes() == before and not journal.exists()
    target.write_bytes(after)
    journal.write_bytes(journal_bytes)
    offline = os.environ.copy()
    offline.update({'HTTPS_PROXY': 'http://127.0.0.1:1', 'https_proxy': 'http://127.0.0.1:1',
                    'NO_PROXY': '', 'no_proxy': ''})
    result = subprocess.run([str(binary), 'update', '--directory', directory, '--json'],
                            env=offline, capture_output=True, text=True)
    assert result.returncode != 0, (result.stdout, result.stderr)
    assert json.loads(result.stdout)['mutation'] == 'rolled_back', result.stdout
    assert target.read_bytes() == before and not journal.exists()
    target.write_bytes(after)
    journal.write_bytes(journal_bytes)
    result = subprocess.run([str(binary), 'update', '--offline', '--directory', directory, '--json'], capture_output=True, text=True)
    envelope = json.loads(result.stdout)
    assert result.returncode == 0 and envelope['data']['no_op'] is True, (result.stdout, result.stderr)
    assert envelope['mutation'] == 'rolled_back'
    assert target.read_bytes() == before and not journal.exists()
print('default packaged build ignored crash/release-root/self-replace test bypass environment')
