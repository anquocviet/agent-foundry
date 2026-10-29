# AgentFoundry

Development build `0.1.0-dev.1`, **not v1 or production-ready**. Full release scope remains [proposal](AgentFoundry-Proposal-v1.md) §24; unfinished capabilities are tracked, not removed.

## Local development

Rust 1.90.0 (pinned), Python 3.10+ (tested locally with 3.12), Git for merge/evidence. No account, telemetry, application startup, or consumer dependency installation.

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 scripts/package-local.py
python3 tests/acceptance/check.py artifacts/0.1.0-dev.1/agentfoundry-linux-x64
python3 -m unittest discover -s tests/evidence -p check.py -v
python3 tests/acceptance/payload.py artifacts/0.1.0-dev.1/agentfoundry-linux-x64
python3 tests/acceptance/no_bypass.py artifacts/0.1.0-dev.1/agentfoundry-linux-x64
cargo build --locked --features fault-injection --target-dir target/fault-injection
python3 tests/acceptance/crashes.py target/fault-injection/debug/agentfoundry --all-files
python3 tests/acceptance/saved_plan.py artifacts/0.1.0-dev.1/agentfoundry-linux-x64
python3 tests/acceptance/inventory.py artifacts/0.1.0-dev.1/agentfoundry-linux-x64
python3 tests/acceptance/doctor.py artifacts/0.1.0-dev.1/agentfoundry-linux-x64
python3 -B examples/workflow_examples.py
python3 -B tests/acceptance/workflow_examples.py
```

`bash scripts/validate-premerge.sh` runs the listed local source/package/evidence gates and five-boundary fault smoke in order, failing on the first error. It is a Linux development gate, not native Windows/macOS CI or the public release gate. Use `crashes.py --all-files` separately for the complete fresh-install checkpoint sweep.

Package script currently targets the local Linux x64 development host; it is not the five-platform public release pipeline. `Cargo.lock` pins the resolved graph. Source identity is a development SHA-256 over declared build inputs, not an immutable Git release claim.

Fault injection is compiled only with the explicit `fault-injection` feature, has a distinct source identity, and is rejected by local packaging. Crash checks exercise interrupted install and rollback in disposable consumers, including preservation of post-commit local edits. They do not establish power-loss durability or native Windows activation safety.

`inspect` reports package scripts as declarations, never executes them, and excludes secret-file discovery. Invalid package root/script types fail with `INVALID_STATE` rather than becoming command suggestions.

Installed `status` includes generation, selected addons/adapters and running executable build identity; offline latest remains `Unknown`. `doctor` rejects a repo-local executable with missing Unix execute bits even when its content digest matches.

`status --check-updates --directory /path/to/consumer --json` explicitly reads release metadata from the owner-approved GitHub repository `anquocviet/agent-foundry`; ordinary status remains offline. The check does not install, execute a candidate or write consumer state. Missing public release returns `RELEASE_UNAVAILABLE`, not a claim that the installed build is latest. Cross-release upgrades and activation remain unavailable.

With a pending transaction, `status` reports a read-only recovery preview or its blocking diagnostic before loading the active manifest; it does not repair state. `doctor` still returns recovery-required until explicitly recovered.

Doctor dependency observations are bounded version probes, not workflow proof. Missing Git/Python is reported as `Unknown` without failing offline maintenance; observed versions also remain `Unknown` until minimum-version and capability verification is implemented.

Offline update captures and revalidates all managed containers, baseline objects, config, manifest and repo-local executable before returning a plan. Snapshot hashes cover exact whole-file bytes and Unix modes; this detects controlled planning drift, not uncooperative-writer isolation. Persisted conflict sessions and cross-release update remain unimplemented.

Snapshot/session control DTOs use strict versioned JSON with required nullable fingerprints and preimages. Parsing validates portable paths and semantic input bindings; snapshot fingerprints must match frozen control entries and conflict LOCAL hashes must match frozen workspace entries. Public conflict staging/continue/abort is still unavailable.

`update --offline --dry-run --plan-out /outside/plan.json` saves an exact embedded-update preview; `update --offline --apply-plan /outside/plan.json` rejects drift and reauthorizes against the same installed build. Saved plans remain inline and limited to 16 MiB; this does not enable cross-release upgrades.

## Synthetic consumer example

Use a new disposable target outside this source repository:

```sh
artifacts/0.1.0-dev.1/agentfoundry-linux-x64 install --directory /tmp/af-example --dry-run --json
artifacts/0.1.0-dev.1/agentfoundry-linux-x64 install --directory /tmp/af-example --json
/tmp/af-example/scripts/bin/agentfoundry status --directory /tmp/af-example --json
```

Implemented: offline inspect/status/doctor, fresh installation/adoption, explicit replacement, marker shims, optional wisdom/Claude, no-op reinstall preserving local changes, snapshot-checked saved install plans, blob-backed journal/backup recovery, Git three-way decision adapter, namespaced protocol and inherited evidence helpers. `update --offline` validates the exact installed embedded build and preserves local edits; it cannot upgrade across releases. Consumer writes go through the transaction engine. Cross-release update/conflict sessions/self-update/bootstrap remain **not implemented** and block release.

Existing collisions require `--merge` (skip consumer-owned differences) or `--override` (replace selected targets with backup). `--refresh-agent-shim` integrates the managed block without replacing outer instructions. Read `--help --json` and [recovery limitations](docs/recovery.md). Do not use this build on valuable repositories.

`examples/workflow_examples.py` runs synthetic invariant positive/negative proofs and a failed baseline/fresh-rerun improvement. It changes only harness invocation, verifies artifact hashes, uses temporary files, and does not claim hook/CI/client enforcement or a real pilot.

`migrate harness --directory PATH` inventories legacy state without writing. `--dry-run --plan-out /outside/migration.json` previews a verified source-baseline or 0.1.10 installation; `--apply-plan /outside/migration.json` rechecks preimages and authorizes the plan before transaction apply. Unknown baselines are read-only. Old state/docs/skills/binaries remain; only recognized entrypoint blocks are replaced, preserving outer bytes. Conflicts stop migration; existing AgentFoundry state is never overwritten. Legacy skills remain discoverable: use only the new `agentfoundry-*` skills after migration; cleanup is a separate user decision.

Local pinned-fixture proof: `python3 tests/acceptance/migration.py artifacts/0.1.0-dev.1/agentfoundry-linux-x64 /path/to/repository-harness-checkout`. Both pinned Git commits must exist locally; no installer or network fetch runs.

[Parity](docs/parity-matrix.md) · [Tests](docs/test-mapping.md) · [Status/checkpoint](docs/implementation-status.md) · [Decisions](docs/architecture-decisions.md) · [Notices](THIRD_PARTY_NOTICES.md)
