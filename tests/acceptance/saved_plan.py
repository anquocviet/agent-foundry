#!/usr/bin/env python3
"""A preview must not publish an unusable or public-readable saved plan."""
from pathlib import Path
import json
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='agentfoundry-saved-plan-') as directory:
    base = Path(directory)
    root = base / 'consumer'
    root.mkdir()
    private = b'private instruction\n' + b'x' * (7 * 1024 * 1024) + b'\n'
    (root / 'AGENTS.md').write_bytes(private)
    output = base / 'oversized.json'
    result = subprocess.run([str(binary), 'install', '--directory', str(root), '--merge',
                             '--refresh-agent-shim', '--dry-run', '--plan-out', str(output), '--json'],
                            capture_output=True, text=True)
    envelope = json.loads(result.stdout)
    assert result.returncode != 0 and envelope['code'] == 'INPUT_LIMIT_EXCEEDED', envelope
    assert not output.exists()
    assert {path.name for path in root.iterdir()} == {'AGENTS.md'}
    assert (root / 'AGENTS.md').read_bytes() == private
    oversized_input = base / 'oversized-input.json'
    with oversized_input.open('wb') as file:
        file.truncate(16 * 1024 * 1024 + 1)
    result = subprocess.run([str(binary), 'install', '--directory', str(root),
                             '--apply-plan', str(oversized_input), '--json'], capture_output=True, text=True)
    assert result.returncode == 5 and json.loads(result.stdout)['code'] == 'INPUT_LIMIT_EXCEEDED'
    assert (root / 'AGENTS.md').read_bytes() == private
    assert not (root / '.agentfoundry').exists()
    boundary = b'x' * (8 * 1024 * 1024)
    (root / 'AGENTS.md').write_bytes(boundary)
    result = subprocess.run([str(binary), 'install', '--directory', str(root), '--merge',
                             '--refresh-agent-shim', '--json'], capture_output=True, text=True)
    assert result.returncode == 5 and json.loads(result.stdout)['code'] == 'INPUT_LIMIT_EXCEEDED'
    assert (root / 'AGENTS.md').read_bytes() == boundary
    assert {path.name for path in root.iterdir()} == {'AGENTS.md'}
    (root / 'AGENTS.md').unlink()
    (root / '.gitignore').write_bytes(boundary)
    result = subprocess.run([str(binary), 'install', '--directory', str(root), '--json'],
                            capture_output=True, text=True)
    assert result.returncode == 5 and json.loads(result.stdout)['code'] == 'INPUT_LIMIT_EXCEEDED'
    assert (root / '.gitignore').read_bytes() == boundary
    assert {path.name for path in root.iterdir()} == {'.gitignore'}
print('oversized saved plan rejected before output or consumer mutation')
