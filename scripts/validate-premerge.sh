#!/usr/bin/env bash
# Local Linux gate; native matrix and release promotion remain separate requirements.
set -euo pipefail
root="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$root"
cargo fmt --all --check
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
python3 scripts/package-local.py
binary=artifacts/0.1.0-dev.1/agentfoundry-linux-x64
python3 tests/acceptance/payload.py "$binary"
python3 tests/acceptance/no_bypass.py "$binary"
python3 tests/acceptance/activation_mutation.py "$binary"
python3 tests/acceptance/saved_plan.py "$binary"
python3 tests/acceptance/inventory.py "$binary"
python3 tests/acceptance/doctor.py "$binary"
python3 tests/acceptance/schemas.py
python3 -B tests/acceptance/workflow_examples.py
python3 -m unittest discover -s tests/evidence -p check.py -v
cargo build --locked --features fault-injection --target-dir target/fault-injection
python3 tests/acceptance/crashes.py target/fault-injection/debug/agentfoundry
