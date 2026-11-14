#!/usr/bin/env python3
"""Validate complete release bytes before and after uploading a draft."""
import hashlib
import json
from pathlib import Path
import re
import sys

root, tag, commit = Path(sys.argv[1]), sys.argv[2], sys.argv[3]
platforms = {'x86_64-apple-darwin', 'aarch64-apple-darwin', 'x86_64-unknown-linux-gnu',
             'aarch64-unknown-linux-gnu', 'x86_64-pc-windows-msvc'}
if not re.fullmatch(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', tag) or not re.fullmatch(r'[0-9a-f]{40}', commit):
    raise ValueError('invalid tag or commit')
metadata = json.loads((root / 'agentfoundry-release.json').read_text(encoding='utf-8'))
if (set(metadata) != {'schema', 'version', 'tag', 'source_commit', 'payload_digest', 'state_schema_min', 'state_schema_max', 'handoff_protocol', 'assets'}
        or metadata['schema'] != 'agentfoundry.release/v1' or metadata['tag'] != tag
        or metadata['version'] != tag[1:] or metadata['source_commit'] != commit
        or not re.fullmatch(r'[0-9a-f]{64}', metadata['payload_digest'])
        or metadata['state_schema_min'] != 1 or metadata['state_schema_max'] != 1 or metadata['handoff_protocol'] != 1):
    raise ValueError('invalid release metadata')
assets = metadata['assets']
if not isinstance(assets, list) or len(assets) != 5 or {a['platform'] for a in assets} != platforms:
    raise ValueError('invalid platform inventory')
names = {'agentfoundry-release.json'}
for asset in assets:
    platform = asset['platform']
    name = 'agentfoundry-' + platform + ('.exe' if platform.endswith('windows-msvc') else '')
    checksum = name + '.sha256'
    if set(asset) != {'platform', 'name', 'size', 'sha256', 'checksum_name'} or asset['name'] != name or asset['checksum_name'] != checksum:
        raise ValueError('invalid asset names')
    binary = root / name
    size = binary.stat().st_size
    if size != asset['size'] or not 0 < size <= 128 * 1024 * 1024:
        raise ValueError('invalid binary size')
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    if digest != asset['sha256'] or (root / checksum).read_bytes() != f'{digest}  {name}\n'.encode():
        raise ValueError('binary or checksum mismatch')
    names.update((name, checksum))
if {p.name for p in root.iterdir()} != names:
    raise ValueError('unexpected release files')
print(f'Verified {tag} at {commit}: five binaries and checksum sidecars')
