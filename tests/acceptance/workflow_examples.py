#!/usr/bin/env python3
"""T-055 acceptance test: verifies runnable invariant guard and improvement fresh rerun."""
import json
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True

workspace_root = Path(__file__).resolve().parents[2]
example_script = workspace_root / "examples/workflow_examples.py"

proc = subprocess.run(
    [sys.executable, "-B", str(example_script)],
    capture_output=True,
    text=True,
)
assert proc.returncode == 0, f"workflow_examples.py failed ({proc.returncode}):\n{proc.stderr}"

output = json.loads(proc.stdout)

# 1. Invariant: real AST positive/negative proof
inv = output["invariant"]
assert inv["positive_violations"] == 0
assert inv["negative_violations"] == 1

# 2. Improvement: baseline failure, one invocation intervention, recomputed exact SHA256
imp = output["improvement"]
assert imp["baseline_exit"] == 2
assert imp["fresh_exit"] == 0
assert imp["fresh_sha256"] == imp["expected_sha256"]

print("workflow examples: invariant positive/negative proof and improvement fresh rerun passed")
