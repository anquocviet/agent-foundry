#!/usr/bin/env python3
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

binary = str(Path(sys.argv[1]).resolve())
digest = lambda data: hashlib.sha256(data).hexdigest()
with tempfile.TemporaryDirectory(prefix='af-activation-mutation-') as directory:
    root = Path(directory).resolve() / 'consumer'
    def run(*args):
        result = subprocess.run([binary, *args, '--directory', str(root), '--json'], capture_output=True, text=True)
        return result, json.loads(result.stdout)
    result, _ = run('install')
    assert result.returncode == 0
    state_path = root / '.agentfoundry/state/manifest.json'
    state = json.loads(state_path.read_bytes())
    original = state_path.read_bytes()
    bin_name = 'scripts/bin/agentfoundry.exe' if sys.platform == 'win32' else 'scripts/bin/agentfoundry'
    target = root / bin_name
    old_binary = target.read_bytes()
    candidate = b'#!/bin/sh\necho invalid-json\n'
    candidate_hash = digest(candidate)
    state['binary_digest'] = candidate_hash
    target_state = json.dumps(state, indent=2).encode()
    state_path.write_bytes(target_state)
    object_path = root / '.agentfoundry/backups/abc/objects' / candidate_hash
    object_path.parent.mkdir(parents=True)
    object_path.write_bytes(candidate)
    root_identity = f'\\\\?\\{root.resolve()}' if sys.platform == 'win32' else f'{root.stat().st_dev}:{root.stat().st_ino}:{root}'
    record = {'schema': 'agentfoundry.activation/v1', 'id': 'abc',
              'root_identity': root_identity,
              'before_state': digest(original), 'after_state': digest(target_state),
              'before_binary': digest(old_binary),
              'target': {'version': state['version'], 'source': state['source'],
                         'payload_digest': state['payload_digest'], 'binary_digest': candidate_hash},
              'release': None, 'mode': 0o755}
    record_path = root / '.agentfoundry/runtime/activation.json'
    record_path.write_text(json.dumps(record))
    target.unlink()
    result, response = run('recover')
    assert result.returncode != 0 and response['mutation'] == 'applied', response
    assert target.read_bytes() == candidate and record_path.exists()
    result, response = run('recover')
    assert result.returncode != 0 and response['mutation'] == 'none', response
    assert target.read_bytes() == candidate and record_path.exists()
print('activation identity failure reports applied only when binary was replaced')
