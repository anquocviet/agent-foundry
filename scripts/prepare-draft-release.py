#!/usr/bin/env python3
"""Create and verify a *draft* release. Non-destructive; never publishes.

Usage: prepare-draft-release.py TAG COMMIT BUNDLE_DIR REPO

The uploaded release is always created with ``--draft``. Drafts are excluded
from the consumer ``releases/latest`` pointer, so an interrupted or invalid run
cannot expose a partial release to consumers. This script is the only
release-mutating step in ``draft-release.yml``; publication (making a draft
public) is a separate, owner-approved action and is deliberately not
implemented here.

This step never deletes anything. Earlier revisions deleted a draft on failure
or before a retry; that is a check-then-delete race against an owner who may
publish the draft manually between the state read and the deletion, and it can
destroy a freshly published release. Instead:

* A pre-existing draft for the tag is retained as evidence and the run refuses.
  Explicit owner recovery (inspect, then delete or promote) is required before
  a retry. Retry needing explicit recovery is intentional, not a defect.
* A published (or prerelease) release for the tag is immutable input; the run
  refuses before any mutation.
* A failure after the create attempt leaves the draft untouched and exits
  nonzero. The draft is not consumer-visible and remains evidence.

No code path here publishes, deletes, or overwrites a release.
"""
import json
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tempfile

TAG_RE = re.compile(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)')
COMMIT_RE = re.compile(r'[0-9a-f]{40}')


def gh(*args, check=True):
    result = subprocess.run(['gh', *args], text=True, capture_output=True)
    if check and result.returncode != 0:
        raise RuntimeError(
            f"gh {' '.join(args)} failed ({result.returncode}): "
            f"{(result.stderr or result.stdout).strip()}"
        )
    return result


def release_state(tag, repo):
    """Return the release view, None only for a confirmed missing release.

    Any other failure is ambiguous and must abort: treating a transient API
    error as "missing" could overwrite a published release on retry.
    """
    result = gh('release', 'view', tag, '--repo', repo, '--json', 'isDraft,tagName,assets', check=False)
    if result.returncode != 0:
        if (result.stderr or '').strip().lower() == 'release not found':
            return None
        raise RuntimeError(
            f"cannot determine release state for {tag}: "
            f"{(result.stderr or result.stdout).strip()}"
        )
    return json.loads(result.stdout)


def main():
    if len(sys.argv) != 5:
        raise SystemExit('usage: prepare-draft-release.py TAG COMMIT BUNDLE_DIR REPO')
    tag, commit, bundle_arg, repo = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
    if not TAG_RE.fullmatch(tag) or not COMMIT_RE.fullmatch(commit):
        raise SystemExit('invalid tag or commit')
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repo):
        raise SystemExit('invalid repository')
    bundle = Path(bundle_arg)

    verifier = Path(__file__).resolve().with_name('verify-release-bundle.py')
    subprocess.run([sys.executable, str(verifier), str(bundle), tag, commit], check=True)
    expected = sorted(p.name for p in bundle.iterdir())
    if not expected:
        raise SystemExit('empty bundle')

    existing = release_state(tag, repo)
    if existing is not None:
        if not existing.get('isDraft'):
            raise SystemExit(
                f'refusing to modify published release {tag}; published assets are immutable'
            )
        raise SystemExit(
            f'refusing to replace pre-existing draft {tag}; '
            'inspect and explicitly delete or promote it before retrying'
        )
    if json.loads(gh('api', f'repos/{repo}/commits/refs/tags/{tag}').stdout)['sha'] != commit:
        raise ValueError('tag does not resolve to approved commit')

    gh('release', 'create', tag, *(str(bundle / name) for name in expected),
       '--repo', repo, '--verify-tag', '--draft', '--title', tag,
       '--notes', 'Draft candidate; publication requires separate owner approval.')

    view = release_state(tag, repo)
    if view is None:
        raise RuntimeError(f'draft {tag} missing after creation')
    if not view.get('isDraft') or view.get('tagName') != tag:
        raise RuntimeError(f'draft {tag} is not a draft of the expected tag')
    uploaded = sorted(asset['name'] for asset in view['assets'])
    if uploaded != expected:
        raise RuntimeError(
            f'uploaded inventory mismatch: expected {len(expected)} assets, got {len(uploaded)}'
        )

    with tempfile.TemporaryDirectory() as tmp:
        # Use the same bounded, ID-pinned downloads and tag checks as promotion.
        verify = runpy.run_path(str(verifier.with_name('promote-release.py')))['verify']
        release, _ = verify(repo, tag, commit, Path(tmp))
        if not release['draft']:
            raise RuntimeError('release was published during draft verification')
        diff = subprocess.run(['diff', '-rq', str(bundle), tmp], text=True, capture_output=True)
        if diff.returncode != 0:
            raise RuntimeError(f'uploaded bytes differ from bundle: {diff.stdout.strip()}')

    # Success leaves a verified draft. Publishing is intentionally absent.
    # ponytail: cooperative writers only; GitHub has no cross-call tag/asset CAS.
    # Freeze tag/asset edits during preparation; this script cannot undo publication.
    print(f'Verified draft {tag} with {len(expected)} assets; still draft, not published')


if __name__ == '__main__':
    main()
