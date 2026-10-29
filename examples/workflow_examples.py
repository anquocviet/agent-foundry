#!/usr/bin/env python3
"""T-055 runnable examples: invariant AST guard and harness invocation improvement."""
import ast
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True

AUTHORITY = "ADR-0001 (Core Domain Isolation)"
FORBIDDEN_MODULES = ("infrastructure", "interface")


def check_domain_ast(source: str, filename: str = "domain.py") -> list[str]:
    """Real AST guard enforcing inward domain isolation."""
    tree = ast.parse(source, filename=filename)
    violations = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                for forbidden in FORBIDDEN_MODULES:
                    if alias.name == forbidden or alias.name.startswith(f"{forbidden}."):
                        violations.append(
                            f"{filename}:{node.lineno}: imports '{alias.name}': violates {AUTHORITY}. "
                            "Inward domain modules must not import outward layers. "
                            "Depend on an inward domain abstraction."
                        )
        elif isinstance(node, ast.ImportFrom):
            mod = node.module or ""
            for forbidden in FORBIDDEN_MODULES:
                if mod == forbidden or mod.startswith(f"{forbidden}."):
                    violations.append(
                        f"{filename}:{node.lineno}: imports '{mod}': violates {AUTHORITY}. "
                        "Inward domain modules must not import outward layers. "
                        "Depend on an inward domain abstraction."
                    )
    return violations


def run_invariant_example() -> dict:
    conforming_src = (
        "from dataclasses import dataclass\n\n"
        "@dataclass(frozen=True)\n"
        "class Entity:\n"
        "    id: str\n"
    )
    pos_violations = check_domain_ast(conforming_src, "src/domain/entity.py")
    assert len(pos_violations) == 0

    violating_src = (
        "from dataclasses import dataclass\n"
        "from infrastructure.database import Connection\n\n"
        "@dataclass\n"
        "class Repo:\n"
        "    conn: Connection\n"
    )
    neg_violations = check_domain_ast(violating_src, "src/domain/repo.py")
    assert len(neg_violations) == 1
    assert "infrastructure.database" in neg_violations[0]
    assert "src/domain/repo.py:2" in neg_violations[0]

    return {
        "status": "passed",
        "authority": AUTHORITY,
        "positive_violations": len(pos_violations),
        "negative_violations": len(neg_violations),
        "negative_diagnostic": neg_violations[0],
    }


def run_improvement_example() -> dict:
    with tempfile.TemporaryDirectory(prefix="agentfoundry-improvement-") as temp_dir:
        worker_script = Path(temp_dir) / "worker.py"
        # Accepted consumer task requires explicit '--profile standard'
        worker_code = (
            "import sys, json, hashlib\n"
            "args = sys.argv[1:]\n"
            "if '--profile' not in args or args[args.index('--profile') + 1] != 'standard':\n"
            "    sys.stderr.write('Task rejected: required --profile standard omitted\\n')\n"
            "    sys.exit(2)\n"
            "payload = {'target': 'build', 'profile': 'standard'}\n"
            "data = json.dumps(payload, sort_keys=True).encode()\n"
            "digest = hashlib.sha256(data).hexdigest()\n"
            "print(json.dumps({'status': 'ok', 'sha256': digest, 'payload': payload}))\n"
        )
        worker_script.write_text(worker_code, encoding="utf-8")
        worker_digest = hashlib.sha256(worker_code.encode()).hexdigest()

        # Baseline: harness invocation omits required --profile flag
        baseline_cmd = [sys.executable, "-B", str(worker_script), "--target", "build"]
        baseline = subprocess.run(baseline_cmd, capture_output=True, text=True)
        assert baseline.returncode == 2

        # One intervention: harness updates invocation to include required flag
        fresh_cmd = [sys.executable, "-B", str(worker_script), "--target", "build", "--profile", "standard"]
        fresh = subprocess.run(fresh_cmd, capture_output=True, text=True)
        assert fresh.returncode == 0

        evidence = json.loads(fresh.stdout)
        expected_payload = {"target": "build", "profile": "standard"}
        expected_digest = hashlib.sha256(json.dumps(expected_payload, sort_keys=True).encode()).hexdigest()
        assert evidence["sha256"] == expected_digest
        assert evidence["payload"] == expected_payload

        return {
            "status": "passed",
            "source_digest": worker_digest,
            "baseline_cmd": baseline_cmd,
            "baseline_exit": baseline.returncode,
            "baseline_stderr": baseline.stderr.strip(),
            "fresh_cmd": fresh_cmd,
            "fresh_exit": fresh.returncode,
            "fresh_sha256": evidence["sha256"],
            "expected_sha256": expected_digest,
            "decision": "keep",
        }


def main():
    print(json.dumps({
        "invariant": run_invariant_example(),
        "improvement": run_improvement_example(),
    }, indent=2))


if __name__ == "__main__":
    main()
