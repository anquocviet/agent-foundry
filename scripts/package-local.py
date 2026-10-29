#!/usr/bin/env python3
"""Package locally built bytes, never download or publish a release."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
subprocess.run([str(Path.home() / '.cargo/bin/cargo'), 'build', '--release', '--locked'], cwd=root, check=True)
source = root / 'target/release/agentfoundry'
identity = json.loads(subprocess.check_output([str(source), '--version', '--json']))['data']
if identity.get('fault_injection') is not False:
    raise RuntimeError('Refusing to package a fault-injection executable')
output = root / 'artifacts' / identity['version']
output.mkdir(parents=True, exist_ok=True)
binary = output / 'agentfoundry-linux-x64'
with tempfile.NamedTemporaryFile(dir=output, prefix='.agentfoundry-', delete=False) as staged:
    staged_path = Path(staged.name)
try:
    shutil.copy2(source, staged_path)
    staged_path.replace(binary)
finally:
    staged_path.unlink(missing_ok=True)
digest = hashlib.sha256(binary.read_bytes()).hexdigest()
(binary.with_suffix('.sha256')).write_text(digest + '  ' + binary.name + '\n')
(output / 'development-metadata.json').write_text(json.dumps({**identity, 'sha256': digest, 'asset': binary.name, 'release_ready': False}, indent=2) + '\n')
subprocess.run(['python3', str(root / 'tests/acceptance/check.py'), str(binary)], cwd=root, check=True)
print(json.dumps({'artifact': str(binary.relative_to(root)), 'sha256': digest, **identity}))
