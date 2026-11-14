#!/usr/bin/env python3
"""Assemble verified native CI build records into a downloadable release bundle."""
import hashlib
import json
from pathlib import Path
import re
import shutil
import sys

PLATFORMS = ('x86_64-apple-darwin', 'aarch64-apple-darwin', 'x86_64-unknown-linux-gnu',
             'aarch64-unknown-linux-gnu', 'x86_64-pc-windows-msvc')
source, output, tag, commit = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3], sys.argv[4]
if not re.fullmatch(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', tag) or not re.fullmatch(r'[0-9a-f]{40}', commit):
    raise ValueError('invalid tag or source commit')
if set(p.name for p in source.iterdir()) != {f'stable-candidate-{p}' for p in PLATFORMS}:
    raise ValueError('expected exactly five platform artifacts')
records = []
for platform in PLATFORMS:
    directory = source / f'stable-candidate-{platform}'
    binary_name = 'agentfoundry.exe' if platform.endswith('windows-msvc') else 'agentfoundry'
    if set(p.name for p in directory.iterdir()) != {binary_name, 'identity.json'}:
        raise ValueError(f'invalid artifact inventory: {platform}')
    identity = json.loads((directory / 'identity.json').read_text(encoding='utf-8'))
    binary = directory / binary_name
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    if (identity.get('platform') != platform or identity.get('version') != tag[1:]
            or identity.get('source') != commit or identity.get('distribution') != 'stable'
            or identity.get('fault_injection') is not False or identity.get('sha256') != digest
            or not re.fullmatch(r'[0-9a-f]{64}', identity.get('payload_digest', ''))
            or not 0 < binary.stat().st_size <= 128 * 1024 * 1024):
        raise ValueError(f'invalid binary identity: {platform}')
    records.append((platform, binary, digest, identity['payload_digest']))
if len({r[3] for r in records}) != 1:
    raise ValueError('payload digest mismatch between platforms')
output.mkdir(parents=True, exist_ok=False)
assets = []
for platform, binary, digest, payload in records:
    name = f'agentfoundry-{platform}' + ('.exe' if platform.endswith('windows-msvc') else '')
    shutil.copyfile(binary, output / name)
    checksum = name + '.sha256'
    (output / checksum).write_text(f'{digest}  {name}\n', encoding='utf-8', newline='\n')
    assets.append(dict(platform=platform, name=name, size=binary.stat().st_size, sha256=digest, checksum_name=checksum))
metadata = dict(schema='agentfoundry.release/v1', version=tag[1:], tag=tag, source_commit=commit,
                payload_digest=records[0][3], state_schema_min=1, state_schema_max=1, handoff_protocol=1, assets=assets)
(output / 'agentfoundry-release.json').write_text(json.dumps(metadata, indent=2) + '\n', encoding='utf-8', newline='\n')
print(f'Prepared {len(assets)} binaries, checksums and metadata; no release published')
