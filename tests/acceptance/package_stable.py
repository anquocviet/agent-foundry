#!/usr/bin/env python3
"""Run with Python: verify release inventory and reject missing or altered artifacts."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

script = Path(__file__).resolve().parents[2] / 'scripts/package-stable.py'
platforms = ('x86_64-apple-darwin', 'aarch64-apple-darwin', 'x86_64-unknown-linux-gnu',
             'aarch64-unknown-linux-gnu', 'x86_64-pc-windows-msvc')
commit, payload = 'a' * 40, 'b' * 64
with tempfile.TemporaryDirectory() as tmp:
    root = Path(tmp)
    source = root / 'input'
    source.mkdir()
    for platform in platforms:
        directory = source / ('stable-candidate-' + platform)
        directory.mkdir()
        name = 'agentfoundry.exe' if platform.endswith('windows-msvc') else 'agentfoundry'
        data = ('binary-' + platform).encode()
        (directory / name).write_bytes(data)
        identity = dict(platform=platform, version='1.2.3', source=commit, distribution='stable',
                        fault_injection=False, sha256=hashlib.sha256(data).hexdigest(), payload_digest=payload)
        (directory / 'identity.json').write_text(json.dumps(identity), encoding='utf-8')

    def package(name):
        return subprocess.run([sys.executable, str(script), str(source), str(root / name), 'v1.2.3', commit],
                              capture_output=True, text=True)

    result = package('bundle')
    assert result.returncode == 0, result.stderr
    bundle = root / 'bundle'
    verify = script.with_name('verify-release-bundle.py')
    def check_bundle():
        return subprocess.run([sys.executable, str(verify), str(bundle), 'v1.2.3', commit], capture_output=True, text=True)
    assert check_bundle().returncode == 0
    metadata = json.loads((bundle / 'agentfoundry-release.json').read_text(encoding='utf-8'))
    assert metadata['source_commit'] == commit and metadata['payload_digest'] == payload
    assert len(metadata['assets']) == 5 and len(list(bundle.iterdir())) == 11
    for asset in metadata['assets']:
        data = (bundle / asset['name']).read_bytes()
        assert len(data) == asset['size'] and hashlib.sha256(data).hexdigest() == asset['sha256']
        assert (bundle / asset['checksum_name']).read_text().split() == [asset['sha256'], asset['name']]
    bundled_binary = bundle / metadata['assets'][0]['name']
    original_bundle = bundled_binary.read_bytes()
    bundled_binary.write_bytes(b'tampered')
    assert check_bundle().returncode != 0
    bundled_binary.write_bytes(original_bundle)
    (bundle / metadata['assets'][0]['checksum_name']).unlink()
    assert check_bundle().returncode != 0

    binary = source / 'stable-candidate-x86_64-pc-windows-msvc' / 'agentfoundry.exe'
    original = binary.read_bytes()
    binary.write_bytes(b'tampered')
    assert package('tampered').returncode != 0 and not (root / 'tampered').exists()
    binary.write_bytes(original)
    missing = source / ('stable-candidate-' + platforms[0])
    missing.rename(root / 'missing-platform')
    assert package('missing').returncode != 0 and not (root / 'missing').exists()
    print('stable bundle, tampering and missing platform checks passed')
