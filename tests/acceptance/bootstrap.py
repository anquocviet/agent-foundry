#!/usr/bin/env python3
"""Exercise both bootstrap wrappers against a verified local stable bundle.

Usage: python3 tests/acceptance/bootstrap.py BUNDLE_DIR [powershell.exe]
The bundle is a staging fixture, not evidence of a production release.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

bundle = Path(sys.argv[1]).resolve()
metadata = json.loads((bundle / 'agentfoundry-release.json').read_text())
release = {'tag_name': metadata['tag'], 'draft': False, 'prerelease': False,
           'assets': [{'name': p.name} for p in bundle.iterdir()]}
repo = Path(__file__).resolve().parents[2]
with tempfile.TemporaryDirectory(prefix='af-bootstrap-test-') as tmp:
    tmp = Path(tmp)
    (tmp / 'release.json').write_text(json.dumps(release))
    shell = (repo / 'scripts/install.sh').read_text()
    start = shell.index('fetch_bounded() {')
    end = shell.index('\n# --- Main Entry Point ---', start)
    shell = shell[:start] + '''fetch_bounded() {
    local name="${1##*/}"
    if [ "$2" = api ]; then name=release.json; fi
    cp "$AF_FIXTURE/$name" "$5"
}
''' + shell[end:]
    script = tmp / 'install.sh'
    script.write_text(shell)
    fixture = tmp / 'fixture'
    fixture.mkdir()
    shutil.copy(tmp / 'release.json', fixture / 'release.json')
    for file in bundle.iterdir():
        shutil.copy(file, fixture / file.name)
    env = dict(os.environ, AF_FIXTURE=str(fixture))
    root = tmp / 'consumer space'
    def run(*args):
        return subprocess.run(['bash', str(script), '--directory', str(root), *args, '--json'], env=env, text=True, capture_output=True)
    preview = run('--dry-run', '--claude')
    assert preview.returncode == 0, (preview.stdout, preview.stderr)
    assert not root.exists()
    installed = run('--claude')
    assert installed.returncode == 0, (installed.stdout, installed.stderr)
    assert (root / 'CLAUDE.md').read_text().count('@AGENTS.md') == 1
    before = (root / 'CLAUDE.md').read_bytes()
    asset = next(a for a in metadata['assets'] if a['platform'] == 'x86_64-unknown-linux-gnu')
    with (fixture / asset['name']).open('ab') as f:
        f.write(b'corrupt')
    bad = run('--override')
    assert bad.returncode != 0 and (root / 'CLAUDE.md').read_bytes() == before
    print('Bash: dry-run, install, Claude forwarding and corrupt candidate refusal passed')

    if len(sys.argv) > 2:
        powershell = sys.argv[2]
        def win(path):
            return subprocess.check_output(['wslpath', '-w', str(path)], text=True).strip().replace("'", "''")
        windows_fixture = win(fixture)
        ps = (repo / 'scripts/install.ps1').read_text()
        start = ps.index('function Invoke-BoundedGet(')
        end = ps.index('\nfunction Validate-Semver(', start)
        ps = ps[:start] + f'''function Invoke-BoundedGet([string]$url, [string]$context, [string]$accept, [long]$maxBytes, [string]$outFilePath = $null) {{
    $name = if ($context -eq 'Api') {{ 'release.json' }} else {{ ($url -split '/')[-1] }}
    $source = Join-Path '{windows_fixture}' $name
    if ($outFilePath) {{ Copy-Item -LiteralPath $source -Destination $outFilePath }} else {{ [IO.File]::ReadAllText($source) }}
}}
''' + ps[end:]
        script = tmp / 'install.ps1'
        script.write_text(ps)
        windows_temp = Path(subprocess.check_output(['wslpath', '-u', subprocess.check_output([powershell, '-NoProfile', '-Command', '[IO.Path]::GetTempPath()'], text=True).strip()], text=True).strip())
        win_root = tempfile.TemporaryDirectory(prefix='af-bootstrap-consumer-', dir=windows_temp)
        root = Path(win_root.name) / 'consumer space'
        def run_ps(*args):
            return subprocess.run([powershell, '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', win(script), '--directory', win(root), *args, '--json'], text=True, capture_output=True)
        preview = run_ps('--dry-run', '--claude')
        assert preview.returncode == 0, (preview.stdout, preview.stderr)
        assert not root.exists()
        installed = run_ps('--claude')
        assert installed.returncode == 0, (installed.stdout, installed.stderr)
        assert (root / 'CLAUDE.md').read_text().count('@AGENTS.md') == 1
        before = (root / 'CLAUDE.md').read_bytes()
        asset = next(a for a in metadata['assets'] if a['platform'] == 'x86_64-pc-windows-msvc')
        with (fixture / asset['name']).open('ab') as f:
            f.write(b'corrupt')
        bad = run_ps('--override')
        assert bad.returncode != 0 and (root / 'CLAUDE.md').read_bytes() == before
        win_root.cleanup()
        print('Windows PowerShell: dry-run, install, Claude forwarding and corrupt candidate refusal passed')
