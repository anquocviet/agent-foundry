# AgentFoundry

Pre-release `0.1.0` candidate preparation, **not v1 or production-ready**. Full release scope remains [proposal](AgentFoundry-Proposal-v1.md) §24; unfinished capabilities are tracked, not removed.

## Local development

Rust 1.90.0 (pinned), Python 3.10+ (tested locally with 3.12), Git for merge/evidence. No account, telemetry, application startup, or consumer dependency installation.

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 scripts/package-local.py
python3 tests/acceptance/check.py artifacts/0.1.0/agentfoundry-linux-x64
python3 -m unittest discover -s tests/evidence -p check.py -v
python3 tests/acceptance/payload.py artifacts/0.1.0/agentfoundry-linux-x64
python3 tests/acceptance/no_bypass.py artifacts/0.1.0/agentfoundry-linux-x64
cargo build --locked --features fault-injection --target-dir target/fault-injection
python3 tests/acceptance/crashes.py target/fault-injection/debug/agentfoundry --all-files
python3 tests/acceptance/saved_plan.py artifacts/0.1.0/agentfoundry-linux-x64
python3 tests/acceptance/inventory.py artifacts/0.1.0/agentfoundry-linux-x64
python3 tests/acceptance/doctor.py artifacts/0.1.0/agentfoundry-linux-x64
python3 -B examples/workflow_examples.py
python3 -B tests/acceptance/workflow_examples.py
```

`bash scripts/validate-premerge.sh` runs the listed local source/package/evidence gates and five-boundary fault smoke in order, failing on the first error. It is a Linux development gate, not native Windows/macOS CI or the public release gate. Use `crashes.py --all-files` separately for the complete fresh-install checkpoint sweep.

`scripts/package-local.py` packages a local binary on native Linux x64 or Windows x64, verifies the public CLI and writes a SHA-256 sidecar. Windows output is `artifacts/0.1.0/agentfoundry-windows-x64.exe`; this is not the five-platform public release pipeline. `Cargo.lock` pins the resolved graph. Local development builds record a development SHA-256 over declared build inputs; only `AF_STABLE_BUILD` from a clean tagged checkout records an immutable Git commit identity.

`.github/workflows/stable-candidate.yml` manually builds five stable candidates from an existing clean matching version tag, verifies their recorded identities and hashes, and uploads a downloadable bundle containing binaries, checksum sidecars and `agentfoundry-release.json`. It cannot publish a release; promotion and release approval remain separate gates. `AF_STABLE_BUILD` rejects fault injection and requires a clean tagged checkout.

`.github/workflows/draft-release.yml` manually takes a stable tag and successful stable-candidate run for that exact commit, checks the downloaded bundle, creates a draft and verifies uploaded bytes. Existing releases are not overwritten; publication requires separate owner approval. A failed upload or post-upload verification can leave an incomplete draft and must not be published.
Bootstrap: `bash scripts/install.sh --directory PATH [install options]` on Linux/macOS (requires Bash, curl, jq and sha256sum or shasum), or `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/install.ps1 --directory PATH [install options]` on Windows. The wrappers acquire and verify the pinned stable release before passing options to core; they fail without creating the target if no release is available. Checksums from the same release origin are not independent publisher authentication. Production bootstrap and interactive PowerShell remain unverified.


`.github/workflows/ci.yml` runs native GitHub-hosted jobs for macOS Intel/arm64, Linux x64/arm64 and Windows x64. Push the workflow to a branch and open a pull request or run it with `workflow_dispatch`; uploaded binaries are development artifacts, not release candidates. Runner availability and successful remote results must be checked before claiming platform coverage.

On Windows, install the MSVC C++ build tools and use a fresh checkout with `.gitattributes` applied: payload bytes must remain LF for manifest hashes. Existing CRLF payload files must be restored as LF without overwriting local edits. Native smoke commands (PowerShell, Python 3.10+):

```powershell
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --release --locked
python scripts/package-local.py
python tests/acceptance/check.py target/release/agentfoundry.exe
python tests/acceptance/payload.py target/release/agentfoundry.exe
cargo build --locked --features fault-injection --target-dir target/fault-injection
python tests/acceptance/crashes.py target/fault-injection/debug/agentfoundry.exe
```

The symlink check reports a skip without Windows symlink privilege. Windows installations include the transactionally managed `scripts/agentfoundry.ps1` launcher. Run `powershell -NoProfile -ExecutionPolicy Bypass -File PATH/scripts/agentfoundry.ps1 update --directory PATH --json`. The launcher waits for a temporary worker checked against the installed executable; if the executable is missing, recovery needs a separately available trusted AgentFoundry binary (`AGENTFOUNDRY_BIN`, launcher-local binary, or PATH). A published update succeeded against an isolated staging endpoint; production release activation remains unverified.

Fault injection is compiled only with the explicit `fault-injection` feature, has a distinct source identity, and is rejected by local packaging. Crash checks exercise interrupted install and rollback in disposable consumers, including preservation of post-commit local edits. They do not establish power-loss durability or Windows self-replacement safety.

`inspect` reports package scripts as declarations, never executes them, and excludes secret-file discovery. Invalid package root/script types fail with `INVALID_STATE` rather than becoming command suggestions.

Installed `status` includes generation, selected addons/adapters and running executable build identity; offline latest remains `Unknown`. `doctor` rejects a repo-local executable with missing Unix execute bits even when its content digest matches.

`status --check-updates --directory /path/to/consumer --json` explicitly reads release metadata from the owner-approved GitHub repository `anquocviet/agent-foundry`; ordinary status remains offline. The check does not install, execute a candidate or write consumer state. Missing public release returns `RELEASE_UNAVAILABLE`, not a claim that the installed build is latest. A positive published update has been verified only against an isolated staging endpoint.

With a pending transaction, `status` reports a read-only recovery preview or its blocking diagnostic before loading the active manifest; it does not repair state. `doctor` still returns recovery-required until explicitly recovered.

Doctor dependency observations are bounded version probes, not workflow proof. Missing Git/Python is reported as `Unknown` without failing offline maintenance; observed versions also remain `Unknown` until minimum-version and capability verification is implemented.

Offline update captures and revalidates all managed containers, baseline objects, config, manifest and repo-local executable before returning a plan. Snapshot hashes cover exact whole-file bytes and Unix modes; this detects controlled planning drift, not uncooperative-writer isolation. Distinct-build conflict staging, continue and abort have development acceptance evidence; the complete tamper, crash and native matrix remains open.

Snapshot/session control DTOs use strict versioned JSON with required nullable fingerprints and preimages. Parsing validates portable paths and semantic input bindings; snapshot fingerprints must match frozen control entries and conflict LOCAL hashes must match frozen workspace entries.

`update --offline --dry-run --plan-out /outside/plan.json` saves an exact embedded-update preview; `update --offline --apply-plan /outside/plan.json` rejects drift and reauthorizes against the same installed build. Saved plans remain inline and limited to 16 MiB; this does not enable cross-release upgrades.

## Synthetic consumer example

Use a new disposable target outside this source repository:

```sh
artifacts/0.1.0/agentfoundry-linux-x64 install --directory /tmp/af-example --dry-run --json
artifacts/0.1.0/agentfoundry-linux-x64 install --directory /tmp/af-example --json
/tmp/af-example/scripts/bin/agentfoundry status --directory /tmp/af-example --json
```

Implemented in development: offline inspect/status/doctor, fresh installation/adoption, explicit replacement, marker shims, optional wisdom/Claude, no-op reinstall preserving local changes, snapshot-checked saved plans, blob-backed journal/backup recovery, Git three-way planning, conflict sessions and executable activation. A published update has been exercised only against an isolated staging release, not production. Bootstrap and release acceptance remain incomplete; see [implementation status](docs/implementation-status.md). Consumer writes go through the transaction engine.

Existing collisions require `--merge` (skip consumer-owned differences) or `--override` (replace selected targets with backup). `--refresh-agent-shim` integrates the managed block without replacing outer instructions. Read `--help --json` and [recovery limitations](docs/recovery.md). Do not use this build on valuable repositories.

`examples/workflow_examples.py` runs synthetic invariant positive/negative proofs and a failed baseline/fresh-rerun improvement. It changes only harness invocation, verifies artifact hashes, uses temporary files, and does not claim hook/CI/client enforcement or a real pilot.

`migrate harness --directory PATH` inventories legacy state without writing. `--dry-run --plan-out /outside/migration.json` previews a verified source-baseline or 0.1.10 installation; `--apply-plan /outside/migration.json` rechecks preimages and authorizes the plan before transaction apply. Unknown baselines are read-only. Old state/docs/skills/binaries remain; only recognized entrypoint blocks are replaced, preserving outer bytes. Conflicts stop migration; existing AgentFoundry state is never overwritten. Legacy skills remain discoverable: use only the new `agentfoundry-*` skills after migration; cleanup is a separate user decision.

Local pinned-fixture proof: `python3 tests/acceptance/migration.py artifacts/0.1.0/agentfoundry-linux-x64 /path/to/repository-harness-checkout`. Both pinned Git commits must exist locally; no installer or network fetch runs.

[Parity](docs/parity-matrix.md) · [Tests](docs/test-mapping.md) · [Status/checkpoint](docs/implementation-status.md) · [Decisions](docs/architecture-decisions.md) · [Notices](THIRD_PARTY_NOTICES.md)
