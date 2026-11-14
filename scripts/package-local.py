#!/usr/bin/env python3
"""Package locally built bytes, never download or publish a release."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import platform
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
host = (sys.platform, platform.machine().lower())
if host in (('linux', 'x86_64'), ('linux', 'amd64')):
    source_name, asset_name = 'agentfoundry', 'agentfoundry-linux-x64'
elif host == ('win32', 'amd64'):
    source_name, asset_name = 'agentfoundry.exe', 'agentfoundry-windows-x64.exe'
else:
    raise RuntimeError(f'Unsupported local packaging host: {host}')
subprocess.run(['cargo', 'build', '--release', '--locked'], cwd=root, check=True)
source = root / 'target/release' / source_name
identity = json.loads(subprocess.check_output([str(source), '--version', '--json']))['data']
if identity.get('fault_injection') is not False:
    raise RuntimeError('Refusing to package a fault-injection executable')
output = root / 'artifacts' / identity['version']
output.mkdir(parents=True, exist_ok=True)
binary = output / asset_name
with tempfile.NamedTemporaryFile(dir=output, prefix='.agentfoundry-', delete=False) as staged:
    staged_path = Path(staged.name)
try:
    shutil.copy2(source, staged_path)
    staged_path.replace(binary)
finally:
    staged_path.unlink(missing_ok=True)
digest = hashlib.sha256(binary.read_bytes()).hexdigest()
(output / (binary.name + '.sha256')).write_text(digest + '  ' + binary.name + '\n')
metadata_name = 'development-metadata.json' if sys.platform == 'linux' else 'development-metadata-windows-x64.json'
(output / metadata_name).write_text(json.dumps({**identity, 'sha256': digest, 'asset': binary.name, 'release_ready': False}, indent=2) + '\n')
subprocess.run([sys.executable, str(root / 'tests/acceptance/check.py'), str(binary)], cwd=root, check=True)
print(json.dumps({'artifact': str(binary.relative_to(root)), 'sha256': digest, **identity}))
