#!/usr/bin/env python3
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='agentfoundry-doctor-') as directory:
    root = Path(directory) / 'consumer'
    subprocess.run([str(binary), 'install', '--directory', str(root), '--json'], check=True, capture_output=True)
    environment = os.environ.copy()
    empty_path = Path(directory) / 'empty-path'
    empty_path.mkdir()
    environment['PATH'] = str(empty_path)
    result = subprocess.run([str(binary), 'doctor', '--directory', str(root), '--json'], env=environment, capture_output=True, text=True)
    assert result.returncode == 0, (result.stdout, result.stderr)
    checks = {check['capability']: check for check in json.loads(result.stdout)['data']['checks']}
    assert set(checks) == {'git', 'python'}
    assert all(check['result'] == 'Unknown' and 'unavailable' in check['reason'] for check in checks.values())
    assert not list(root.rglob('__pycache__'))
    for arguments in (['doctor', '--check-updates'], ['status', '--check-updates', '--offline']):
        invalid = subprocess.run([str(binary), *arguments, '--directory', str(root), '--json'], capture_output=True, text=True)
        assert invalid.returncode == 2 and json.loads(invalid.stdout)['code'] == 'INVALID_ARGUMENT'
    offline_status = subprocess.run([str(binary), 'status', '--directory', str(root), '--json'], env=environment, capture_output=True, text=True)
    assert offline_status.returncode == 0
    status_data = json.loads(offline_status.stdout)['data']
    assert status_data['latest'] == 'Unknown' and 'release_observation' not in status_data
print('doctor: missing optional Git/Python reported Unknown without failing offline maintenance')
