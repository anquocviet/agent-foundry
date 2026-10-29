#!/usr/bin/env python3
import json
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='agentfoundry-inspect-') as directory:
    root = Path(directory)
    manifest = root / 'package.json'
    manifest.write_text(json.dumps({'scripts': {'test': 'touch NEVER_EXECUTE'}}))
    (root / '.env').write_text('PRIVATE_TOKEN=must-not-appear')
    before = {p.name: p.read_bytes() for p in root.iterdir()}
    result = subprocess.run([str(binary), 'inspect', '--directory', directory, '--json'], capture_output=True, text=True, check=True)
    data = json.loads(result.stdout)['data']
    assert data['commands_executed'] is False and data['network'] is False
    assert data['sources'][0]['declared_commands'] == {'test': 'touch NEVER_EXECUTE'}
    assert 'PRIVATE_TOKEN' not in result.stdout and '.env' not in result.stdout
    assert {p.name: p.read_bytes() for p in root.iterdir()} == before
    for invalid in ([], {'scripts': []}, {'scripts': {'test': 42}}):
        manifest.write_text(json.dumps(invalid))
        result = subprocess.run([str(binary), 'inspect', '--directory', directory, '--json'], capture_output=True, text=True)
        assert result.returncode == 5 and json.loads(result.stdout)['code'] == 'INVALID_STATE'
        assert not (root / 'NEVER_EXECUTE').exists()
print('inspect: declared commands only, no secret reads/output or command execution; invalid script types rejected')
