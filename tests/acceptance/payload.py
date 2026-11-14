#!/usr/bin/env python3
"""Manifest/hash/link proof on actual installed payload, never source as consumer."""
import hashlib
import json
from pathlib import Path
import posixpath
import re
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
source = Path(__file__).resolve().parents[2]
manifest = json.loads((source / 'payload/manifest.json').read_text())
with tempfile.TemporaryDirectory(prefix='agentfoundry-payload-') as directory:
    root = Path(directory)
    subprocess.run([str(binary), 'install', '--directory', str(root), '--with-engineering-wisdom', '--claude', '--json'], check=True, capture_output=True)
    entries = {e['target']: e for e in manifest['entries']}
    installed = json.loads((root / '.agentfoundry/state/manifest.json').read_text())
    assert {entry['target'] for entry in installed['entries']} == set(entries)
    actual = {path.relative_to(root).as_posix() for path in root.rglob('*') if path.is_file()
              and not path.is_relative_to(root / '.agentfoundry')
              and not path.is_relative_to(root / 'scripts/bin')}
    assert actual == set(entries) | {'.gitignore'}, (actual - set(entries) - {'.gitignore'}, set(entries) - actual)
    for target, entry in entries.items():
        original = (source / entry['source']).read_bytes()
        assert hashlib.sha256(original).hexdigest() == entry['sha256']
        assert (root / target).read_bytes() == original, target
        if not target.endswith('.md'):
            continue
        text = original.decode('utf-8')
        for link in re.findall(r'\[[^\]]*\]\(([^)]+)\)', text):
            if '://' in link or link.startswith(('#', 'mailto:')):
                continue
            link = link.split('#', 1)[0]
            normalized = posixpath.normpath(posixpath.join(posixpath.dirname(target), link))
            assert not normalized.startswith('../'), (target, link)
            assert (root / normalized).exists(), (target, link, normalized)
print('installed payload exact bytes and explicit Markdown links passed')
