#!/usr/bin/env python3
"""Deterministic fault harness for T-058: interrupted draft preparation.

This check runs the *actual* production orchestration
(``scripts/prepare-draft-release.py``) and the *actual* bundle verifier
(``scripts/verify-release-bundle.py``) against a truthful local simulation of
the ``gh`` CLI. Only the external GitHub boundary is simulated: the simulator
models drafts, published releases, per-asset uploads, the ``releases/latest``
consumer pointer, a concurrent manual owner publication, and interruption
between persisted uploads. The bundle itself is produced by the real
``scripts/package-stable.py``.

It does not touch any remote repository, staging or production. Every scenario
is a local simulation; no evidence about live GitHub behaviour is claimed
beyond the documented draft semantics.

Proved properties (non-destructive draft preparation):
  * a draft (complete or partial) is never returned by the latest pointer;
  * interruption during upload, verification or download never publishes and
    never leaves a consumer-visible release;
  * failed or interrupted runs leave the draft as evidence and the script never
    deletes anything; a retry refuses a pre-existing draft instead of racing a
    manual owner publication;
  * explicit owner recovery (delete the draft out of band) followed by a retry
    produces exactly the 11 verified assets, still a draft;
  * a published release for the tag is refused and never overwritten;
  * a manual owner publication during the state read survives intact because
    the script issues no delete;
  * no code path invokes a publish command (there is no promotion contract).
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
PACKAGE = ROOT / 'scripts/package-stable.py'
PREPARE = ROOT / 'scripts/prepare-draft-release.py'
PLATFORMS = ('x86_64-apple-darwin', 'aarch64-apple-darwin', 'x86_64-unknown-linux-gnu',
             'aarch64-unknown-linux-gnu', 'x86_64-pc-windows-msvc')
TAG = 'v1.2.3'
PREVIOUS = 'v1.2.2'
COMMIT = 'a' * 40
REPO = 'owner/repo'

SHIM = r'''#!/usr/bin/env python3
import hashlib, json, os, shutil, sys
from pathlib import Path

state_dir = Path(os.environ['AF058_GH_STATE_DIR'])
state_path = state_dir / 'state.json'
faults_path = state_dir / 'faults.json'
log_path = state_dir / 'calls.jsonl'
assets_root = state_dir / 'assets'


def load_state():
    if state_path.exists():
        return json.loads(state_path.read_text())
    return {'releases': [], 'seq': 0}


def save_state(state):
    state_path.write_text(json.dumps(state))


def faults():
    if faults_path.exists():
        return json.loads(faults_path.read_text())
    return {}


def log(entry):
    with log_path.open('a') as handle:
        handle.write(json.dumps(entry) + '\n')


def find(state, tag):
    for release in state['releases']:
        if release['tag_name'] == tag:
            return release
    return None


def die(message, code=1):
    print(message, file=sys.stderr)
    sys.exit(code)


def option(args, name):
    if name in args:
        return args[args.index(name) + 1]
    return None


def cmd_view(args):
    tag = args[0]
    state = load_state()
    if faults().get('fail_view_always'):
        die('HTTP 502: Bad Gateway')
    if faults().get('ambiguous_not_found'):
        die('proxy cache not found; upstream state unknown')
    release = find(state, tag)
    if release is None:
        die('release not found')
    if faults().get('fail_view'):
        die('HTTP 502: Bad Gateway')
    payload = {
        'isDraft': release['draft'],
        'tagName': release['tag_name'],
        'assets': [{'name': asset['name']} for asset in release['assets']],
    }
    # Model an owner publishing manually between this read and any later
    # action. The reader still receives the stale draft view.
    if faults().get('publish_after_view') and release['draft']:
        release['draft'] = False
        save_state(state)
        log({'cmd': 'manual_publish', 'tag': tag})
    print(json.dumps(payload))


def cmd_create(args):
    tag = args[0]
    files = []
    index = 1
    while index < len(args):
        arg = args[index]
        if arg in ('--repo', '--title', '--notes'):
            index += 2
            continue
        if arg.startswith('--'):
            index += 1
            continue
        files.append(arg)
        index += 1
    state = load_state()
    if find(state, tag) is not None:
        die('release already exists')
    draft = '--draft' in args
    log({'cmd': 'create', 'tag': tag, 'draft': draft,
         'files': [Path(path).name for path in files]})
    if not draft:
        log({'cmd': 'publish', 'tag': tag, 'via': 'create'})
    state['seq'] += 1
    release = {'tag_name': tag, 'draft': draft, 'prerelease': False,
               'seq': state['seq'], 'assets': []}
    state['releases'].append(release)
    save_state(state)
    config = faults()
    fail_after = config.get('fail_create_after_uploads')
    drop_after = config.get('create_silent_drop_after')
    corrupt = config.get('corrupt_upload')
    destination = assets_root / tag
    destination.mkdir(parents=True, exist_ok=True)
    uploaded = 0
    for path in files:
        source = Path(path)
        data = source.read_bytes()
        if corrupt == source.name:
            data = bytes(byte ^ 0xFF for byte in data[:16]) + data[16:]
        (destination / source.name).write_bytes(data)
        release['assets'].append({
            'name': source.name,
            'sha256': hashlib.sha256(data).hexdigest(),
            'size': len(data),
        })
        uploaded += 1
        save_state(state)
        if fail_after is not None and uploaded >= fail_after:
            die('interrupted while uploading assets', 130)
        if drop_after is not None and uploaded >= drop_after:
            return 0
    return 0


def cmd_delete(args):
    tag = args[0]
    if faults().get('fail_delete'):
        die('connection reset by peer')
    state = load_state()
    release = find(state, tag)
    if release is None:
        die('release not found')
    log({'cmd': 'delete', 'tag': tag, 'draft': release['draft']})
    state['releases'].remove(release)
    save_state(state)
    shutil.rmtree(assets_root / tag, ignore_errors=True)
    return 0


def cmd_api(args):
    endpoint = args[0]
    state = load_state()
    config = faults()
    if '/commits/' in endpoint:
        assert endpoint.endswith('/commits/refs/tags/v1.2.3')
        print(json.dumps({'sha': 'd' * 40 if config.get('wrong_tag_commit') else 'a' * 40}))
        return
    if '/releases/assets/' in endpoint:
        identifier = int(endpoint.rsplit('/', 1)[1])
        for release in state['releases']:
            for index, asset in enumerate(release['assets']):
                if identifier != release['seq'] * 100 + index:
                    continue
                if config.get('download_silent_drop') is not None and index >= config['download_silent_drop']:
                    return
                data = (assets_root / release['tag_name'] / asset['name']).read_bytes()
                sys.stdout.buffer.write(data)
                sys.stdout.buffer.flush()
                if config.get('fail_download_after') is not None and index + 1 >= config['fail_download_after']:
                    die('download interrupted', 130)
                return
        die('asset not found')
    if '/releases/tags/' in endpoint:
        release = find(state, endpoint.rsplit('/', 1)[1])
    else:
        release = next((r for r in state['releases'] if str(r['seq']) == endpoint.rsplit('/', 1)[1]), None)
    if release is None:
        die('release not found')
    print(json.dumps(dict(id=release['seq'], tag_name=release['tag_name'],
                         draft=release['draft'], prerelease=release['prerelease'],
                         assets=[dict(id=release['seq'] * 100 + index, name=a['name'],
                                      size=a['size'], updated_at='fixed')
                                 for index, a in enumerate(release['assets'])])))


def main():
    args = sys.argv[1:]
    if not args:
        die('missing command', 2)
    log({'cmd': 'invoke', 'argv': args})
    if args[0] == 'api':
        return cmd_api(args[1:])
    if args[0] == 'release' and len(args) >= 2:
        sub, rest = args[1], args[2:]
        if sub == 'view':
            return cmd_view(rest)
        if sub == 'create':
            return cmd_create(rest)
        if sub == 'delete':
            return cmd_delete(rest)
        if sub == 'edit':
            log({'cmd': 'publish', 'tag': rest[0] if rest else '', 'via': 'edit'})
            die('publishing is not available in this harness')
    die('unsupported gh invocation: ' + ' '.join(args), 2)


if __name__ == '__main__':
    sys.exit(main() or 0)
'''


def bundle(root, tag, commit, salt):
    source = root / ('source-' + salt)
    source.mkdir()
    for platform in PLATFORMS:
        directory = source / ('stable-candidate-' + platform)
        directory.mkdir()
        name = 'agentfoundry.exe' if platform.endswith('windows-msvc') else 'agentfoundry'
        data = (f'binary-{salt}-{platform}').encode()
        (directory / name).write_bytes(data)
        identity = dict(platform=platform, version=tag[1:], source=commit,
                        distribution='stable', fault_injection=False,
                        sha256=hashlib.sha256(data).hexdigest(), payload_digest='b' * 64)
        (directory / 'identity.json').write_text(json.dumps(identity), encoding='utf-8')
    output = root / ('bundle-' + salt)
    subprocess.run([sys.executable, str(PACKAGE), str(source), str(output), tag, commit],
                   check=True, capture_output=True, text=True)
    return output


def save_state(state_dir, state):
    (state_dir / 'state.json').write_text(json.dumps(state))


def load_state(state_dir):
    path = state_dir / 'state.json'
    if path.exists():
        return json.loads(path.read_text())
    return {'releases': [], 'seq': 0}


def seed(state_dir, tag, bundle_dir, published=True):
    state = load_state(state_dir)
    state['releases'] = [r for r in state['releases'] if r['tag_name'] != tag]
    state['seq'] += 1
    release = {'tag_name': tag, 'draft': not published, 'prerelease': False,
               'seq': state['seq'], 'assets': []}
    destination = state_dir / 'assets' / tag
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True)
    for path in sorted(bundle_dir.iterdir()):
        data = path.read_bytes()
        (destination / path.name).write_bytes(data)
        release['assets'].append({'name': path.name,
                                  'sha256': hashlib.sha256(data).hexdigest(),
                                  'size': len(data)})
    state['releases'].append(release)
    save_state(state_dir, state)


def recover_delete(state_dir, tag):
    """Explicit owner recovery: remove the draft out of band, not via the script."""
    state = load_state(state_dir)
    state['releases'] = [r for r in state['releases'] if r['tag_name'] != tag]
    save_state(state_dir, state)
    shutil.rmtree(state_dir / 'assets' / tag, ignore_errors=True)


def setup(root):
    state_dir = root / 'state'
    state_dir.mkdir()
    bin_dir = root / 'bin'
    bin_dir.mkdir()
    shim = bin_dir / 'gh'
    shim.write_text(SHIM)
    shim.chmod(0o755)
    (state_dir / 'calls.jsonl').touch()
    return state_dir, bin_dir


def run(state_dir, bin_dir, bundle_dir, faults, tag=TAG):
    (state_dir / 'faults.json').write_text(json.dumps(faults))
    environment = dict(os.environ)
    environment['PATH'] = str(bin_dir) + os.pathsep + environment.get('PATH', '')
    environment['AF058_GH_STATE_DIR'] = str(state_dir)
    environment['GH_TOKEN'] = 'fixture-token'
    return subprocess.run(
        [sys.executable, str(PREPARE), tag, COMMIT, str(bundle_dir), REPO],
        capture_output=True, text=True, env=environment)


def target(state_dir, tag=TAG):
    for release in load_state(state_dir)['releases']:
        if release['tag_name'] == tag:
            return release
    return None


def latest(state_dir):
    published = [r for r in load_state(state_dir)['releases']
                 if not r['draft'] and not r['prerelease']]
    if not published:
        return None
    return max(published, key=lambda release: release['seq'])


def calls(state_dir):
    text = (state_dir / 'calls.jsonl').read_text()
    return [json.loads(line) for line in text.splitlines() if line]


def assert_invisible(state_dir, tag=TAG):
    pointer = latest(state_dir)
    assert pointer is None or pointer['tag_name'] != tag, \
        f'latest pointer exposes target release {tag} before promotion: {pointer}'
    releases = [r for r in load_state(state_dir)['releases'] if r['tag_name'] == tag]
    assert all(r['draft'] for r in releases), f'target {tag} became non-draft: {releases}'


def assert_latest_complete(state_dir):
    pointer = latest(state_dir)
    if pointer is None:
        return
    names = [asset['name'] for asset in pointer['assets']]
    assert len(names) == 11 and len(set(names)) == 11, \
        f'consumer-visible release has an incomplete duplicate inventory: {names}'


def assert_draft_inventory(state_dir, bundle_dir, tag=TAG):
    release = target(state_dir, tag)
    assert release is not None and release['draft'], f'{tag} is not a draft: {release}'
    expected = {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                for path in bundle_dir.iterdir()}
    uploaded = {asset['name']: asset['sha256'] for asset in release['assets']}
    assert uploaded == expected, (
        f'asset inventory/digest mismatch\n expected={expected}\n uploaded={uploaded}')
    on_disk = state_dir / 'assets' / tag
    assert {p.name for p in on_disk.iterdir()} == set(expected)
    for name, digest in expected.items():
        assert hashlib.sha256((on_disk / name).read_bytes()).hexdigest() == digest


def assert_retained(state_dir, count, tag=TAG):
    release = target(state_dir, tag)
    assert release is not None and release['draft'], f'{tag} evidence was not retained: {release}'
    assert len(release['assets']) == count, f'{tag} evidence changed: {release}'


def assert_non_destructive(state_dir):
    """The script must never delete or publish anything."""
    entries = calls(state_dir)
    deletes = [c for c in entries if c.get('cmd') == 'delete']
    assert not deletes, f'destructive delete was invoked: {deletes}'
    publish = [c for c in entries
               if c.get('cmd') == 'publish'
               or (c.get('cmd') == 'create' and not c.get('draft', True))]
    assert not publish, f'publish/promotion command was invoked: {publish}'
    creates = [c for c in entries if c.get('cmd') == 'create']
    assert all(c.get('draft') for c in creates), f'non-draft create: {creates}'


def scenario_success(root):
    state_dir, bin_dir = setup(root)
    previous = bundle(root, PREVIOUS, 'c' * 40, 'previous')
    seed(state_dir, PREVIOUS, previous, published=True)
    current = bundle(root, TAG, COMMIT, 'current')
    result = run(state_dir, bin_dir, current, {})
    assert result.returncode == 0, f'orchestration failed: {result.stderr}'
    assert latest(state_dir)['tag_name'] == PREVIOUS, 'latest pointer moved to the draft'
    assert_draft_inventory(state_dir, current)
    assert_invisible(state_dir)
    assert_non_destructive(state_dir)
    return 'draft verified; latest pointer unchanged; no delete/publish'


def scenario_interrupt_retains_and_retry_refuses(root):
    state_dir, bin_dir = setup(root)
    current = bundle(root, TAG, COMMIT, 'current')
    result = run(state_dir, bin_dir, current, {'fail_create_after_uploads': 5})
    assert result.returncode != 0, 'interrupted upload unexpectedly succeeded'
    assert_retained(state_dir, 5)
    assert_invisible(state_dir)
    assert_non_destructive(state_dir)
    evidence = {a['name']: a['sha256'] for a in target(state_dir)['assets']}
    retry = run(state_dir, bin_dir, current, {})
    assert retry.returncode != 0, 'retry silently replaced a pre-existing draft'
    assert_retained(state_dir, 5)
    assert {a['name']: a['sha256'] for a in target(state_dir)['assets']} == evidence
    assert_invisible(state_dir)
    assert_non_destructive(state_dir)
    return 'interruption evidence retained; retry refused without mutation'


def scenario_explicit_recovery_then_retry(root):
    state_dir, bin_dir = setup(root)
    current = bundle(root, TAG, COMMIT, 'current')
    result = run(state_dir, bin_dir, current, {'fail_create_after_uploads': 5})
    assert result.returncode != 0 and target(state_dir) is not None
    recover_delete(state_dir, TAG)
    assert target(state_dir) is None
    retry = run(state_dir, bin_dir, current, {})
    assert retry.returncode == 0, f'retry after explicit recovery failed: {retry.stderr}'
    assert_draft_inventory(state_dir, current)
    assert_invisible(state_dir)
    assert_non_destructive(state_dir)
    return 'explicit owner recovery then retry verified'


def scenario_silent_partial_create_retained(root):
    state_dir, bin_dir = setup(root)
    current = bundle(root, TAG, COMMIT, 'current')
    result = run(state_dir, bin_dir, current, {'create_silent_drop_after': 5})
    assert result.returncode != 0, 'inventory mismatch was not detected'
    assert_retained(state_dir, 5)
    assert_invisible(state_dir)
    assert_non_destructive(state_dir)
    return 'silent partial create detected; partial draft retained as evidence'


def scenario_corrupt_upload_retained(root):
    state_dir, bin_dir = setup(root)
    current = bundle(root, TAG, COMMIT, 'current')
    name = 'agentfoundry-x86_64-unknown-linux-gnu'
    result = run(state_dir, bin_dir, current, {'corrupt_upload': name})
    assert result.returncode != 0, 'corrupted upload bytes were not detected'
    assert_retained(state_dir, 11)
    assert_invisible(state_dir)
    assert_non_destructive(state_dir)
    uploaded = {a['name']: a['sha256'] for a in target(state_dir)['assets']}
    expected = hashlib.sha256((current / name).read_bytes()).hexdigest()
    assert uploaded[name] != expected, 'corruption was not persisted as evidence'
    return 'corrupted uploaded bytes detected; draft retained, not deleted'


def scenario_silent_download_drop_retained(root):
    state_dir, bin_dir = setup(root)
    current = bundle(root, TAG, COMMIT, 'current')
    result = run(state_dir, bin_dir, current, {'download_silent_drop': 10})
    assert result.returncode != 0, 'incomplete download was not detected'
    assert_retained(state_dir, 11)
    assert_invisible(state_dir)
    assert_non_destructive(state_dir)
    return 'incomplete downloaded set detected; unverified draft retained'


def scenario_published_same_tag_refused(root):
    state_dir, bin_dir = setup(root)
    current = bundle(root, TAG, COMMIT, 'current')
    seed(state_dir, TAG, current, published=True)
    before = {a['name']: a['sha256'] for a in target(state_dir)['assets']}
    result = run(state_dir, bin_dir, current, {})
    assert result.returncode != 0, 'published release was not refused'
    after = target(state_dir)
    assert not after['draft'], 'published release was replaced by a draft'
    assert {a['name']: a['sha256'] for a in after['assets']} == before, \
        'published release assets were modified'
    assert latest(state_dir)['tag_name'] == TAG
    assert_non_destructive(state_dir)
    return 'published assets refused and unchanged'


def scenario_manual_publish_race(root):
    state_dir, bin_dir = setup(root)
    current = bundle(root, TAG, COMMIT, 'current')
    seed(state_dir, TAG, current, published=False)  # pre-existing draft
    result = run(state_dir, bin_dir, current, {'publish_after_view': True})
    assert result.returncode != 0, 'pre-existing draft was not refused'
    release = target(state_dir)
    assert release is not None and not release['draft'], \
        f'manual owner publication was lost: {release}'
    expected = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                for p in current.iterdir()}
    assert {a['name']: a['sha256'] for a in release['assets']} == expected, \
        'manually published assets were altered'
    assert latest(state_dir)['tag_name'] == TAG
    assert_latest_complete(state_dir)
    assert_non_destructive(state_dir)
    return 'manual publication during state read survived; no delete issued'


def scenario_ambiguous_view(root):
    state_dir, bin_dir = setup(root)
    previous = bundle(root, PREVIOUS, 'c' * 40, 'previous')
    seed(state_dir, PREVIOUS, previous, published=True)
    current = bundle(root, TAG, COMMIT, 'current')
    for fault in ('fail_view_always', 'ambiguous_not_found'):
        result = run(state_dir, bin_dir, current, {fault: True})
        assert result.returncode != 0, 'ambiguous view failure was not fatal'
        assert target(state_dir) is None, 'ambiguous view still created a release'
        assert_non_destructive(state_dir)
    retry = run(state_dir, bin_dir, current, {})
    assert retry.returncode == 0, f'retry after ambiguous view failed: {retry.stderr}'
    assert_draft_inventory(state_dir, current)
    assert latest(state_dir)['tag_name'] == PREVIOUS
    assert_invisible(state_dir)
    assert_non_destructive(state_dir)
    return 'ambiguous state failed closed; retry verified'


def scenario_wrong_tag_commit(root):
    state_dir, bin_dir = setup(root)
    current = bundle(root, TAG, COMMIT, 'current')
    result = run(state_dir, bin_dir, current, {'wrong_tag_commit': True})
    assert result.returncode != 0, 'wrong tag commit was accepted'
    assert target(state_dir) is None, 'draft created for the wrong tag commit'
    assert not any(c.get('cmd') == 'create' for c in calls(state_dir))
    assert_non_destructive(state_dir)
    return 'wrong tag commit refused before creation'


def scenario_negative_control(root):
    """Prove the consumer-safety assertion itself detects a published partial."""
    state_dir, _ = setup(root)
    save_state(state_dir, {'releases': [{
        'tag_name': TAG, 'draft': False, 'prerelease': False, 'seq': 1,
        'assets': [{'name': 'agentfoundry-x86_64-unknown-linux-gnu',
                    'sha256': '0' * 64, 'size': 1}]}], 'seq': 1})
    try:
        assert_latest_complete(state_dir)
    except AssertionError:
        return 'guard rejects a published partial release'
    raise AssertionError('consumer-safety guard failed to catch a published partial release')


SCENARIOS = (
    ('success_draft_and_latest_pointer', scenario_success),
    ('interrupt_retains_and_retry_refuses', scenario_interrupt_retains_and_retry_refuses),
    ('explicit_recovery_then_retry', scenario_explicit_recovery_then_retry),
    ('silent_partial_create_retained', scenario_silent_partial_create_retained),
    ('corrupt_upload_retained', scenario_corrupt_upload_retained),
    ('silent_download_drop_retained', scenario_silent_download_drop_retained),
    ('published_same_tag_refused', scenario_published_same_tag_refused),
    ('manual_publish_race_no_delete', scenario_manual_publish_race),
    ('ambiguous_view_fails_closed', scenario_ambiguous_view),
    ('wrong_tag_commit_before_create', scenario_wrong_tag_commit),
    ('negative_control_published_partial', scenario_negative_control),
)


def main():
    failures = []
    for name, scenario in SCENARIOS:
        with tempfile.TemporaryDirectory(prefix='af058-') as tmp:
            try:
                outcome = scenario(Path(tmp))
            except AssertionError as error:
                failures.append((name, str(error)))
                print(f'FAIL {name}: {error}')
            else:
                print(f'PASS {name}: {outcome}')
    if failures:
        print(f'{len(failures)} scenario(s) failed')
        return 1
    print(f'release promotion fault harness passed: {len(SCENARIOS)} scenarios')
    return 0


if __name__ == '__main__':
    sys.exit(main())
