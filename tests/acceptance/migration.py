#!/usr/bin/env python3
"""Local migration proof from an explicitly supplied pinned upstream Git checkout."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
upstream = Path(sys.argv[2]).resolve()
commits = ('485fd4062be1c71b27c1c9e5585ac4cd2faa7e06', 'e765792b635b4d5e3e5fc0578f82f9ca5dea2681')

def source(commit, path):
    return subprocess.check_output(['git', '-C', str(upstream), 'show', f'{commit}:{path}'])

def snapshot(root):
    return {p.relative_to(root).as_posix(): p.read_bytes() for p in root.rglob('*') if p.is_file()}

def run(root, *args, code=0):
    result = subprocess.run([str(binary), 'migrate', 'harness', '--directory', str(root), '--json', *args], capture_output=True, text=True)
    assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
    return json.loads(result.stdout)

with tempfile.TemporaryDirectory(prefix='af-migration-') as temp:
    base = Path(temp)
    for index, commit in enumerate(commits):
        root = base / str(index)
        fault_binary = Path(sys.argv[3]).resolve() if len(sys.argv) > 3 else None
        root.mkdir()
        files = []
        listing = source(commit, 'scripts/harness-install-files.txt').decode()
        if index:
            listing += '\n' + source(commit, 'scripts/engineering-wisdom-install-files.txt').decode()
        for path in listing.splitlines():
            if not path or path.startswith('#'):
                continue
            if path == 'AGENTS.md':
                data = b'# Agent Instructions\n\n' + source(commit, 'scripts/agent-harness-block.md')
            elif path in ('docs/plans/README.md', 'docs/plans/completed/README.md', 'docs/decisions/README.md'):
                data = source(commit, 'crates/harness/assets/' + path)
            else:
                data = source(commit, path)
            for target in (root / path, root / '.harness-core/base' / path):
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
            files.append({'path': path, 'upstream_sha256': hashlib.sha256(data).hexdigest()})
        manifest = root / '.harness-core/manifest.json'
        manifest.write_text(json.dumps({'schema_version': 1, 'core_version': '0.1.10' if index else commit, 'files': files}))
        # Consumer entrypoint is an installed block, not upstream development instructions.
        outer = b'consumer instructions\r\n'
        tail = b'\r\nprivate tail'
        (root / 'AGENTS.md').write_bytes(outer + source(commit, 'scripts/agent-harness-block.md').rstrip(b'\n') + tail)
        if index:
            (root / 'CLAUDE.md').write_bytes(outer + source(commit, 'scripts/claude-harness-block.md').rstrip(b'\n') + tail)
        (root / 'legacy.db').write_bytes(b'consumer database sentinel')
        # A local edit in an otherwise unchanged generic file must survive mapping.
        edited_path = 'docs/templates/decision.md'
        edited = (root / edited_path).read_bytes() + b'\nConsumer decision guidance.\n'
        (root / edited_path).write_bytes(edited)
        before = snapshot(root)
        report = run(root)['data']
        assert report['known_baseline'], report
        run(root, '--dry-run')
        assert snapshot(root) == before
        if index:
            claude_path = root / 'CLAUDE.md'
            claude_bytes = claude_path.read_bytes()
            for broken in (b'<!-- HARNESS:BEGIN -->unterminated', b'<!-- HARNESS:END -->orphan'):
                claude_path.write_bytes(b'private\n' + broken)
                malformed_claude = snapshot(root)
                run(root, '--apply', code=5)
                assert snapshot(root) == malformed_claude
            claude_path.write_bytes(claude_bytes)
        agent_path = root / 'AGENTS.md'
        agent_bytes = agent_path.read_bytes()
        nested = agent_bytes.replace(b'<!-- HARNESS:BEGIN -->', b'<!-- HARNESS:BEGIN --><!-- AGENTFOUNDRY:BEGIN -->private<!-- AGENTFOUNDRY:END -->')
        agent_path.write_bytes(nested)
        malformed = snapshot(root)
        run(root, '--apply', code=5)
        assert snapshot(root) == malformed
        agent_path.write_bytes(agent_bytes)
        baseline_file = root / '.harness-core/base/docs/templates/decision.md'
        baseline_bytes = baseline_file.read_bytes()
        baseline_file.write_bytes(b'corrupt baseline')
        corrupted = snapshot(root)
        run(root, '--apply', code=5)
        assert snapshot(root) == corrupted
        baseline_file.write_bytes(baseline_bytes)
        preserved_source = (root / edited_path).read_bytes()
        (root / edited_path).unlink()
        missing_source = snapshot(root)
        run(root, '--apply', code=3)
        assert snapshot(root) == missing_source
        (root / edited_path).write_bytes(preserved_source)
        divergent = root / 'docs/WORKFLOW.md'
        original_workflow = divergent.read_bytes()
        divergent.write_bytes(original_workflow + b'\nConsumer workflow note.\n')
        import os
        missing_git_env = os.environ.copy()
        missing_git_env['PATH'] = str(base / 'unavailable-programs')
        before_missing_git = snapshot(root)
        unavailable = subprocess.run([str(binary), 'migrate', 'harness', '--directory', str(root), '--apply', '--json'], env=missing_git_env, capture_output=True, text=True)
        assert unavailable.returncode == 6 and json.loads(unavailable.stdout)['code'] == 'MISSING_CAPABILITY', (unavailable.stdout, unavailable.stderr)
        assert snapshot(root) == before_missing_git
        broken_git_env = os.environ.copy()
        broken_git_env['GIT_CONFIG_COUNT'] = 'invalid'
        failed_merge = subprocess.run([str(binary), 'migrate', 'harness', '--directory', str(root), '--apply', '--json'], env=broken_git_env, capture_output=True, text=True)
        assert failed_merge.returncode == 1 and json.loads(failed_merge.stdout)['code'] == 'IO_ERROR', (failed_merge.stdout, failed_merge.stderr)
        assert snapshot(root) == before_missing_git
        divergent.write_bytes(b'consumer binary\x00workflow')
        binary_conflict = snapshot(root)
        run(root, '--apply', code=3)
        assert snapshot(root) == binary_conflict
        divergent.write_bytes(original_workflow)
        plan = base / f'plan-{index}.json'
        run(root, '--dry-run', '--plan-out', str(plan))
        if fault_binary and not index:
            import os
            env = os.environ.copy()
            env['AGENTFOUNDRY_TEST_CRASH'] = 'applied:AGENTS.md'
            import shutil
            retry_root = base / 'automatic-migration-recovery'
            shutil.copytree(root, retry_root)
            crashed = subprocess.run([str(fault_binary), 'migrate', 'harness', '--directory', str(root), '--apply', '--json'], env=env, capture_output=True, text=True)
            assert crashed.returncode == 86, (crashed.stdout, crashed.stderr)
            recovered = subprocess.run([str(fault_binary), 'recover', '--directory', str(root), '--json'], capture_output=True, text=True)
            assert recovered.returncode == 0, (recovered.stdout, recovered.stderr)
            assert json.loads(recovered.stdout)['mutation'] == 'rolled_back'
            for path, data in before.items():
                assert (root / path).read_bytes() == data, path
            assert not (root / '.agentfoundry/state/manifest.json').exists()
            crashed_retry = subprocess.run([str(fault_binary), 'migrate', 'harness', '--directory', str(retry_root), '--apply', '--json'], env=env, capture_output=True, text=True)
            assert crashed_retry.returncode == 86, (crashed_retry.stdout, crashed_retry.stderr)
            interrupted_snapshot = snapshot(retry_root)
            run(retry_root, '--dry-run', code=8)
            assert snapshot(retry_root) == interrupted_snapshot
            run(retry_root, '--apply')
            assert not (retry_root / '.agentfoundry/runtime/transaction.json').exists()
            assert (retry_root / '.agentfoundry/state/manifest.json').is_file()
            for path, data in before.items():
                if path not in ('AGENTS.md', 'CLAUDE.md'):
                    assert (retry_root / path).read_bytes() == data, path
        original = (root / edited_path).read_bytes()
        (root / edited_path).write_bytes(original + b'editor after preview\n')
        drifted = snapshot(root)
        run(root, '--apply-plan', str(plan), code=4)
        assert snapshot(root) == drifted
        (root / edited_path).write_bytes(original)
        run(root, '--apply-plan', str(plan))
        after = snapshot(root)
        for path, data in before.items():
            if path not in ('AGENTS.md', 'CLAUDE.md'):
                assert after[path] == data, path
        agents = (root / 'AGENTS.md').read_bytes()
        assert agents.startswith(outer) and agents.endswith(tail)
        assert b'<!-- HARNESS:BEGIN -->' not in agents
        assert b'<!-- AGENTFOUNDRY:BEGIN -->' in agents
        assert (root / '.agentfoundry/state/manifest.json').is_file()
        if index:
            claude = (root / 'CLAUDE.md').read_bytes()
            assert claude.startswith(outer) and claude.endswith(tail)
            assert b'<!-- HARNESS:BEGIN -->' not in claude
            assert b'<!-- AGENTFOUNDRY:CLAUDE:BEGIN -->' in claude
            assert (root / '.agents/skills/agentfoundry-engineering-wisdom/SKILL.md').is_file()
        assert (root / 'docs/agentfoundry/templates/decision.md').read_bytes().endswith(b'Consumer decision guidance.\n')
        stable = snapshot(root)
        run(root)
        assert snapshot(root) == stable
    unknown = base / 'unknown'
    (unknown / '.harness-core').mkdir(parents=True)
    (unknown / '.harness-core/manifest.json').write_text(json.dumps({'schema_version': 1, 'core_version': 'unknown', 'files': []}))
    before = snapshot(unknown)
    assert not run(unknown)['data']['known_baseline']
    run(unknown, '--apply', code=6)
    assert snapshot(unknown) == before
print('public migration: both pinned baselines, preview/apply, legacy bytes, outer instructions and unknown refusal passed')