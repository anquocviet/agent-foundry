#!/usr/bin/env python3
"""Run with packaged binary path; only synthetic temporary consumers are mutated."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
def run(root, *args, exit=0):
    p = subprocess.run([str(binary), *args, '--directory', str(root), '--json'], capture_output=True, text=True)
    assert p.returncode == exit, (args, p.returncode, p.stdout, p.stderr)
    envelope = json.loads(p.stdout)
    assert envelope['schema'] == 'agentfoundry.command-result/v1'
    return envelope

def snapshot(root):
    return {p.relative_to(root).as_posix(): (p.read_bytes(), p.stat().st_mode) for p in root.rglob('*') if p.is_file()}

with tempfile.TemporaryDirectory(prefix='agentfoundry-acceptance-') as temp:
    base = Path(temp).resolve()
    root = base / 'consumer'
    sentinel = base / 'outside'
    sentinel.write_bytes(b'untouched')
    run(root, 'install', '--dry-run')
    assert not root.exists(), 'dry-run created target'
    installed = run(root, 'install')
    assert installed['mutation'] == 'applied'
    state = json.loads((root / '.agentfoundry/state/manifest.json').read_text())
    for e in state['entries']:
        blob = root / '.agentfoundry/state/objects' / e['sha256']
        assert hashlib.sha256(blob.read_bytes()).hexdigest() == e['sha256']
        if e['ownership'] == 'managed_file':
            assert (root / e['target']).read_bytes() == blob.read_bytes()
    bin_target = 'scripts/bin/agentfoundry.exe' if sys.platform == 'win32' else 'scripts/bin/agentfoundry'
    assert (root / bin_target).read_bytes() == binary.read_bytes()
    manifest_path = root / '.agentfoundry/state/manifest.json'
    manifest_bytes = manifest_path.read_bytes()
    for identity_field in ('version', 'source'):
        invalid_state = json.loads(manifest_bytes)
        invalid_state[identity_field] = ''
        manifest_path.write_text(json.dumps(invalid_state))
        invalid_snapshot = snapshot(root)
        run(root, 'status', exit=5)
        run(root, 'install', exit=5)
        assert snapshot(root) == invalid_snapshot
    manifest_path.write_bytes(manifest_bytes)
    before = snapshot(root)
    assert run(root, 'install')['data']['no_op']
    assert snapshot(root) == before
    managed = root / 'docs/agentfoundry/WORKFLOW.md'
    managed.write_bytes(managed.read_bytes() + b'\nconsumer note\n')
    edited = managed.read_bytes()
    run(root, 'install')
    assert managed.read_bytes() == edited
    run(root, 'install', '--refresh-agent-shim')
    assert managed.read_bytes() == edited
    assert 'docs/agentfoundry/WORKFLOW.md' in run(root, 'status')['data']['local_modifications']
    run(root, 'doctor')
    before_update = snapshot(root)
    assert run(root, 'update', '--offline', '--dry-run')['data']['no_op']
    assert run(root, 'update', '--offline')['data']['no_op']
    assert snapshot(root) == before_update
    assert managed.read_bytes() == edited
    update_plan = base / 'update-plan.json'
    run(root, 'update', '--offline', '--dry-run', '--plan-out', str(update_plan))
    assert snapshot(root) == before_update
    tampered_update = json.loads(update_plan.read_bytes())
    tampered_update['operations'][0]['after'] = b'not authorized'.hex()
    tampered_path = base / 'tampered-update.json'
    tampered_path.write_text(json.dumps(tampered_update))
    run(root, 'update', '--offline', '--apply-plan', str(tampered_path), exit=5)
    assert snapshot(root) == before_update
    assert run(root, 'update', '--offline', '--apply-plan', str(update_plan))['data']['no_op']
    omitted_snapshot = json.loads(update_plan.read_bytes())
    omitted_snapshot.pop('snapshot')
    omitted_path = base / 'omitted-snapshot.json'
    omitted_path.write_text(json.dumps(omitted_snapshot))
    run(root, 'update', '--offline', '--apply-plan', str(omitted_path), exit=5)
    assert snapshot(root) == before_update
    detached = json.loads(update_plan.read_bytes())
    detached['operations'][0]['before'] = b'detached input'.hex()
    detached_path = base / 'detached-snapshot-preimage.json'
    detached_path.write_text(json.dumps(detached))
    run(root, 'update', '--offline', '--apply-plan', str(detached_path), exit=5)
    assert snapshot(root) == before_update
    for mutation in ('missing', 'extra', 'duplicate'):
        invalid = json.loads(update_plan.read_bytes())
        entries = invalid['snapshot']['entries']
        if mutation == 'missing':
            entries.pop()
        elif mutation == 'extra':
            entries.append({'path': 'unrelated.md', 'presence': {'kind': 'absent'}})
        else:
            entries.append(entries[0])
        invalid_path = base / f'snapshot-{mutation}.json'
        invalid_path.write_text(json.dumps(invalid))
        run(root, 'update', '--offline', '--apply-plan', str(invalid_path), exit=5)
        assert snapshot(root) == before_update
    config_path = root / '.agentfoundry/config.json'
    config_before = config_path.read_bytes()
    config_path.write_bytes(config_before + b'\n')
    config_drift = snapshot(root)
    run(root, 'update', '--offline', '--apply-plan', str(update_plan), exit=4)
    assert snapshot(root) == config_drift
    config_path.write_bytes(config_before)
    managed.write_bytes(edited + b'editor after preview\n')
    update_drift = snapshot(root)
    run(root, 'update', '--offline', '--apply-plan', str(update_plan), exit=4)
    assert snapshot(root) == update_drift
    managed.write_bytes(edited)
    managed.unlink()
    missing_update = snapshot(root)
    run(root, 'update', '--offline', '--apply-plan', str(update_plan), exit=4)
    assert snapshot(root) == missing_update
    managed.write_bytes(edited)
    entrypoint = root / 'AGENTS.md'
    entrypoint_bytes = entrypoint.read_bytes()
    entrypoint.write_bytes(b'consumer instructions without managed block\n')
    missing_block = snapshot(root)
    run(root, 'update', '--offline', '--dry-run', exit=3)
    assert snapshot(root) == missing_block
    run(root, 'update', '--offline', exit=3)
    assert snapshot(root) == missing_block
    entrypoint.write_bytes(entrypoint_bytes)
    result = run(root, 'update', exit=6 if sys.platform == 'win32' else 7)
    if sys.platform == 'win32':
        assert result['code'] == 'MISSING_CAPABILITY'
    before = snapshot(root)
    run(root, 'install', '--override', '--dry-run')
    assert snapshot(root) == before
    generation_before = json.loads((root / '.agentfoundry/state/manifest.json').read_text())['generation']
    run(root, 'install', '--override')
    assert json.loads((root / '.agentfoundry/state/manifest.json').read_text())['generation'] == generation_before + 1
    assert managed.read_bytes() != edited
    assert any(p.read_bytes() == edited for p in (root / '.agentfoundry/backups').glob('*/objects/*'))
    after_override = snapshot(root)
    assert run(root, 'install')['data']['no_op']
    assert snapshot(root) == after_override
    run(root, '--nonsense', exit=2)
    hostile = base / 'hostile'
    hostile.mkdir()
    try:
        (hostile / 'docs').symlink_to(base, target_is_directory=True)
    except OSError as error:
        if sys.platform != 'win32' or error.winerror != 1314:
            raise
        print('SKIP: Windows symlink privilege unavailable')
    else:
        run(hostile, 'install', exit=5)
        assert sentinel.read_bytes() == b'untouched'
    brown = base / 'brownfield'
    brown.mkdir()
    (brown / 'AGENTS.md').write_bytes(b'private instructions\r\n')
    run(brown, 'install', '--merge', '--refresh-agent-shim', '--claude')
    assert (brown / 'AGENTS.md').read_bytes().startswith(b'private instructions\r\n')
    assert (brown / 'CLAUDE.md').read_text().count('@AGENTS.md') == 1
    unknown = base / 'unknown'
    unknown.mkdir()
    (unknown / 'AGENTS.md').write_text('private')
    run(unknown, 'install', exit=2)
    assert (unknown / 'AGENTS.md').read_text() == 'private'
    assert not (unknown / '.agentfoundry').exists()
    unmanaged = base / 'unmanaged-docs'
    target = unmanaged / 'docs/agentfoundry/WORKFLOW.md'
    target.parent.mkdir(parents=True)
    target.write_bytes(b'consumer protocol\r\n')
    business = unmanaged / 'docs/ARCHITECTURE.md'
    business.write_bytes(b'private application architecture\r\n')
    run(unmanaged, 'install', '--merge')
    unmanaged_state = json.loads((unmanaged / '.agentfoundry/state/manifest.json').read_bytes())
    assert not any(entry['target'] == 'docs/agentfoundry/WORKFLOW.md' for entry in unmanaged_state['entries'])
    assert target.read_bytes() == b'consumer protocol\r\n'
    assert business.read_bytes() == b'private application architecture\r\n'
    consumer_snapshot = snapshot(unmanaged)
    run(unmanaged, 'install', '--merge')
    assert snapshot(unmanaged) == consumer_snapshot
    malformed = base / 'malformed-marker'
    malformed.mkdir()
    (malformed / 'AGENTS.md').write_bytes(b'private\r\n<!-- AGENTFOUNDRY:BEGIN -->unterminated')
    malformed_before = snapshot(malformed)
    run(malformed, 'install', '--merge', '--refresh-agent-shim', exit=5)
    assert snapshot(malformed) == malformed_before
    assert not (malformed / '.agentfoundry').exists()
    ignore_target = base / 'ignore-drift'
    ignore_target.mkdir()
    ignore_path = ignore_target / '.gitignore'
    ignore_path.write_bytes(b'private-original\r\n')
    ignore_plan = base / 'ignore-plan.json'
    run(ignore_target, 'install', '--dry-run', '--plan-out', str(ignore_plan))
    ignore_path.write_bytes(b'private-editor-change\r\n')
    run(ignore_target, 'install', '--apply-plan', str(ignore_plan), exit=4)
    assert ignore_path.read_bytes() == b'private-editor-change\r\n'
    assert not (ignore_target / '.agentfoundry').exists()
    orphan = base / 'orphan-state'
    run(orphan, 'install')
    (orphan / '.agentfoundry/state/manifest.json').unlink()
    orphan_before = snapshot(orphan)
    run(orphan, 'install', '--override', exit=5)
    assert snapshot(orphan) == orphan_before
    run(orphan, 'status', exit=5)
    run(orphan, 'doctor', exit=5)
    assert snapshot(orphan) == orphan_before
    bad_selection = base / 'bad-installed-selection'
    run(bad_selection, 'install')
    bad_manifest_path = bad_selection / '.agentfoundry/state/manifest.json'
    bad_manifest = json.loads(bad_manifest_path.read_bytes())
    bad_manifest['entries'][0]['adapter'] = 'untrusted'
    bad_manifest_path.write_text(json.dumps(bad_manifest))
    tampered_before = snapshot(bad_selection)
    run(bad_selection, 'doctor', exit=5)
    run(bad_selection, 'install', '--override', exit=5)
    assert snapshot(bad_selection) == tampered_before
    bad_manifest['entries'][0].pop('adapter')
    for ownership in ('unknown', 'consumer_owned', 'executable', 'seed_once'):
        bad_manifest['entries'][0]['ownership'] = ownership
        bad_manifest_path.write_text(json.dumps(bad_manifest))
        tampered_before = snapshot(bad_selection)
        run(bad_selection, 'doctor', exit=5)
        run(bad_selection, 'install', '--override', exit=5)
        assert snapshot(bad_selection) == tampered_before
    nested_markers = base / 'nested-markers'
    nested_markers.mkdir()
    (nested_markers / 'AGENTS.md').write_bytes(b'private\r\n<!-- AGENTFOUNDRY:BEGIN --><!-- AGENTFOUNDRY:CLAUDE:BEGIN -->local<!-- AGENTFOUNDRY:CLAUDE:END --><!-- AGENTFOUNDRY:END -->tail')
    nested_before = snapshot(nested_markers)
    run(nested_markers, 'install', '--refresh-agent-shim', exit=5)
    assert snapshot(nested_markers) == nested_before
    preserved = base / 'preserved'
    preserved.mkdir()
    (preserved / 'AGENTS.md').write_bytes(b'consumer owned')
    preserve_plan = base / 'preserve.json'
    run(preserved, 'install', '--merge', '--dry-run', '--plan-out', str(preserve_plan))
    (preserved / 'AGENTS.md').write_bytes(b'consumer edited after planning')
    run(preserved, 'install', '--apply-plan', str(preserve_plan), exit=4)
    assert (preserved / 'AGENTS.md').read_bytes() == b'consumer edited after planning'
    assert not (preserved / '.agentfoundry').exists()
    merged = run(preserved, 'install', '--merge')
    assert any('ENTRYPOINT_NOT_INTEGRATED' in warning for warning in merged['warnings'])
    installed_entries = json.loads((preserved / '.agentfoundry/state/manifest.json').read_text())['entries']
    assert not any(entry['target'] == 'AGENTS.md' for entry in installed_entries)
    planned = base / 'planned'
    plan_file = base / 'install-plan.json'
    preview_id = run(planned, 'install', '--dry-run', '--plan-out', str(plan_file))['data']['plan_id']
    assert run(planned, 'install', '--dry-run')['data']['plan_id'] == preview_id
    assert not planned.exists()
    malformed = json.loads(plan_file.read_bytes())
    malformed.pop('choices')
    malformed_path = base / 'missing-choices.json'
    malformed_path.write_text(json.dumps(malformed))
    run(planned, 'install', '--apply-plan', str(malformed_path), exit=5)
    assert not planned.exists()
    missing_identity = json.loads(plan_file.read_bytes())
    missing_identity.pop('root_identity')
    missing_identity_path = base / 'missing-root-identity.json'
    missing_identity_path.write_text(json.dumps(missing_identity))
    run(planned, 'install', '--apply-plan', str(missing_identity_path), exit=5)
    assert not planned.exists()
    if sys.platform != 'win32':
        assert plan_file.stat().st_mode & 0o777 == 0o600
    assert run(planned, 'install', '--apply-plan', str(plan_file))['data']['plan_id'] == preview_id
    assert run(planned, 'status')['data']['installed']
    # A plan authorized against the previous generation must not be replayed.
    generation_root = base / 'generation-race'
    run(generation_root, 'install')
    (generation_root / 'docs/agentfoundry/WORKFLOW.md').write_bytes(b'consumer generation edit\n')
    generation_plan = base / 'generation-plan.json'
    run(generation_root, 'install', '--override', '--dry-run', '--plan-out', str(generation_plan))
    before_generation = json.loads((generation_root / '.agentfoundry/state/manifest.json').read_bytes())['generation']
    run(generation_root, 'install', '--apply-plan', str(generation_plan))
    generation_state = generation_root / '.agentfoundry/state/manifest.json'
    assert json.loads(generation_state.read_bytes())['generation'] == before_generation + 1
    after_first = snapshot(generation_root)
    stale_result = run(generation_root, 'install', '--apply-plan', str(generation_plan), exit=4)
    assert stale_result['code'] == 'WORKSPACE_DRIFT' and stale_result['mutation'] == 'none'
    assert snapshot(generation_root) == after_first
    # Launch competing applies from the same frozen generation.
    concurrent_root = base / 'concurrent-generation'
    run(concurrent_root, 'install')
    (concurrent_root / 'docs/agentfoundry/WORKFLOW.md').write_bytes(b'concurrent consumer edit\n')
    concurrent_plan = base / 'concurrent-plan.json'
    run(concurrent_root, 'install', '--override', '--dry-run', '--plan-out', str(concurrent_plan))
    expected_workflow = bytes.fromhex(next(op['after'] for op in json.loads(concurrent_plan.read_bytes())['operations'] if op['target'] == 'docs/agentfoundry/WORKFLOW.md'))
    concurrent_state = concurrent_root / '.agentfoundry/state/manifest.json'
    generation = json.loads(concurrent_state.read_bytes())['generation']
    command = [str(binary), 'install', '--apply-plan', str(concurrent_plan), '--directory', str(concurrent_root), '--json']
    processes = [subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for _ in range(2)]
    results = [(process.returncode, stdout, stderr) for process in processes for stdout, stderr in [process.communicate(timeout=60)]]
    outcomes = [(code, json.loads(stdout)) for code, stdout, stderr in results]
    assert sum(code == 0 and value['mutation'] == 'applied' for code, value in outcomes) == 1, results
    assert all(code == 0 or (value['code'] in ('LOCK_BUSY', 'WORKSPACE_DRIFT', 'RECOVERY_REQUIRED') and value['mutation'] == 'none') for code, value in outcomes), results
    assert json.loads(concurrent_state.read_bytes())['generation'] == generation + 1
    assert (concurrent_root / 'docs/agentfoundry/WORKFLOW.md').read_bytes() == expected_workflow
    assert not (concurrent_root / '.agentfoundry/runtime/transaction.json').exists()
    if sys.platform != 'win32':
        import shutil
        original_root = base / 'root-identity'
        original_root.mkdir()
        identity_plan = base / 'root-identity-plan.json'
        run(original_root, 'install', '--dry-run', '--plan-out', str(identity_plan))
        retained_root = base / 'retained-root'
        original_root.rename(retained_root)
        shutil.copytree(retained_root, original_root)
        replaced_snapshot = snapshot(original_root)
        run(original_root, 'install', '--apply-plan', str(identity_plan), exit=4)
        assert snapshot(original_root) == replaced_snapshot
        assert not (original_root / '.agentfoundry').exists()
    stale = base / 'stale'
    stale_plan = base / 'stale.json'
    run(stale, 'install', '--dry-run', '--plan-out', str(stale_plan))
    stale.mkdir()
    (stale / 'AGENTS.md').write_bytes(b'editor input')
    result = run(stale, 'install', '--apply-plan', str(stale_plan), exit=4)
    assert (stale / 'AGENTS.md').read_bytes() == b'editor input'
    assert not (stale / '.agentfoundry').exists()
    optional = base / 'optional'
    run(optional, 'install', '--with-engineering-wisdom', '--claude')
    config_path = optional / '.agentfoundry/config.json'
    selected_config = config_path.read_bytes()
    assert json.loads(selected_config)['addons'] == ['engineering-wisdom']
    assert run(optional, 'install')['data']['no_op']
    assert config_path.read_bytes() == selected_config
    assert (optional / '.agents/skills/agentfoundry-engineering-wisdom/SKILL.md').is_file()
    status = run(optional, 'status')['data']
    assert status['addons'] == ['engineering-wisdom']
    assert set(status['adapters']) == {'agents-md', 'claude'}
    assert status['running_executable']['source'] == status['source']
    assert status['latest'] == 'Unknown'
    if sys.platform != 'win32':
        installed_binary = optional / 'scripts/bin/agentfoundry'
        binary_mode = installed_binary.stat().st_mode & 0o777
        installed_binary.chmod(binary_mode & ~0o111)
        run(optional, 'doctor', exit=5)
        installed_binary.chmod(binary_mode)
    bad_config = json.loads(selected_config)
    bad_config['schema'] = 'agentfoundry.config/v999'
    config_path.write_text(json.dumps(bad_config))
    damaged_snapshot = snapshot(optional)
    run(optional, 'install', '--override', exit=5)
    run(optional, 'doctor', exit=5)
    assert snapshot(optional) == damaged_snapshot
    recovery = base / 'recovery'
    recovery.mkdir()
    plan_bytes = plan_file.read_bytes()
    frozen = json.loads(plan_bytes)
    rec_root = f'\\\\?\\{recovery.resolve()}' if sys.platform == 'win32' else str(recovery)
    rec_identity = rec_root if sys.platform == 'win32' else f'{recovery.stat().st_dev}:{recovery.stat().st_ino}:{recovery}'
    frozen['root'] = rec_root
    first = frozen['operations'][0].copy()
    stored_ops = []
    objects = recovery / '.agentfoundry/backups/abc/objects'
    objects.mkdir(parents=True)
    for op in frozen['operations']:
        stored = dict(op)
        for kind in ('before', 'after'):
            if op[kind] is not None:
                image = bytes.fromhex(op[kind])
                digest = hashlib.sha256(image).hexdigest()
                (objects / digest).write_bytes(image)
                stored[kind] = digest
        stored_ops.append(stored)
    frozen = {'schema': 'agentfoundry.stored-plan/v1', 'root': rec_root,
        'root_identity': rec_identity,
        'payload_digest': frozen['payload_digest'], 'choices': frozen['choices'], 'operations': stored_ops}
    compact = json.dumps(frozen, separators=(',', ':'), ensure_ascii=False).encode()
    backup = recovery / '.agentfoundry/backups/abc/plan.json'
    backup.parent.mkdir(parents=True, exist_ok=True)
    backup.write_bytes(compact)
    journal = recovery / '.agentfoundry/runtime/transaction.json'
    journal.parent.mkdir(parents=True)
    journal.write_text(json.dumps({'schema': 'agentfoundry.journal/v1', 'id': 'abc',
        'root': rec_root, 'plan_hash': hashlib.sha256(compact).hexdigest(),
        'phase': 'Applying', 'plan': frozen}, ensure_ascii=False))
    changed = recovery / first['target']
    changed.parent.mkdir(parents=True, exist_ok=True)
    changed.write_bytes(bytes.fromhex(first['after']))
    changed.chmod(first['mode'])
    before_recover = snapshot(recovery)
    run(recovery, 'recover', '--dry-run')
    assert snapshot(recovery) == before_recover
    assert run(recovery, 'recover')['mutation'] == 'rolled_back'
    assert not changed.exists()
    assert backup.is_file() and not journal.exists()
    assert run(recovery, 'recover')['data']['recovery'] == 'not_needed'
print('public CLI: fresh/install no-op/local edits/dry-run/adoption/Claude/path sentinel/JSON passed')
