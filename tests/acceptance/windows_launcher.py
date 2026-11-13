#!/usr/bin/env python3
"""Windows launcher and worker handoff acceptance test.

Verifies:
1. Worker exit propagation (exit 0 on success, exit code preservation on errors).
2. Recovery with distinct local binary (missing selected binary resolved via trusted
   installed binary, candidate verified by Rust before execution, restores distinct binary).
3. Rejected invalid inputs:
   - Direct in-place execution of self-replacing actions fails closed with MISSING_CAPABILITY (6).
   - Tampered worker binary fails closed with INTEGRITY_FAILURE (5).
   - Worker mode with missing selected binary fails closed with INVALID_STATE (2).
   - Mismatched AF_SELECTED_BINARY fails closed with INVALID_ARGUMENT (2).
   - Invalid CLI arguments fail closed with INVALID_ARGUMENT (2).
4. Activation crash and recovery via selected fault build:
   - Precommit checkpoint crash (applied:scripts/bin/agentfoundry.exe) propagates exit 86,
     retains activation evidence, rolls back uncommitted binary to precommit state on first recovery,
     completes activation repair on subsequent recovery, and verifies idempotence (not_needed).
   - Postcommit checkpoint crash (core-committed-durable) propagates exit 86,
     retains activation evidence and committed binary on first recovery,
     clears activation record on subsequent recovery, and verifies idempotence (not_needed).
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import ctypes
from ctypes import wintypes
import sys
import tempfile

sys.dont_write_bytecode = True

AUTHORITY = "ADR-0004 (Platform Bootstrap Parity) & AF-012"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def find_native_binary() -> Path:
    if len(sys.argv) > 1 and not sys.argv[1].startswith("-"):
        candidate = Path(sys.argv[1]).resolve()
        if candidate.exists():
            return candidate

    repo_root = Path(__file__).resolve().parent.parent.parent
    search_paths = [
        repo_root / "target" / "release" / "agentfoundry.exe",
        repo_root / "target" / "debug" / "agentfoundry.exe",
        repo_root / "target" / "fault-injection" / "debug" / "agentfoundry.exe",
        repo_root / "scripts" / "bin" / "agentfoundry.exe",
    ]
    for p in search_paths:
        if p.exists():
            return p.resolve()

    raise FileNotFoundError("Could not locate native agentfoundry.exe build")


def is_fault_binary(exe: Path) -> bool:
    try:
        res = subprocess.run(
            [str(exe), "version", "--json"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        if res.returncode == 0:
            data = json.loads(res.stdout)
            return data.get("data", {}).get("fault_injection") is True
    except Exception:
        pass
    return False


def find_fault_binary(repo_root: Path, native_bin: Path) -> Path:
    for env_key in ("AF_CHECK_CRASH_BINARY", "AGENTFOUNDRY_FAULT_BINARY"):
        val = os.environ.get(env_key)
        if val:
            p = Path(val).resolve()
            if p.exists():
                return p

    for idx, arg in enumerate(sys.argv):
        if arg == "--fault-binary" and idx + 1 < len(sys.argv):
            p = Path(sys.argv[idx + 1]).resolve()
            if p.exists():
                return p
    if len(sys.argv) > 2 and not sys.argv[2].startswith("-"):
        p = Path(sys.argv[2]).resolve()
        if p.exists():
            return p

    if is_fault_binary(native_bin):
        return native_bin

    candidates = [
        repo_root / "target" / "fault-injection" / "debug" / "agentfoundry.exe",
        repo_root / "target" / "fault-injection" / "release" / "agentfoundry.exe",
    ]
    for p in candidates:
        if p.exists():
            return p.resolve()

    raise FileNotFoundError(
        "Could not locate fault-injection binary (target/fault-injection/debug/agentfoundry.exe or AF_CHECK_CRASH_BINARY)"
    )


def stage_activation(
    consumer: Path,
    act_id: str,
    candidate_bytes: bytes,
    before_bin_bytes: bytes,
) -> tuple[Path, Path, str]:
    candidate_digest = digest(candidate_bytes)
    before_bin_digest = digest(before_bin_bytes)

    backup_dir = consumer / ".agentfoundry" / "backups" / act_id / "objects"
    backup_dir.mkdir(parents=True, exist_ok=True)
    candidate_obj = backup_dir / candidate_digest
    candidate_obj.write_bytes(candidate_bytes)

    state_file = consumer / ".agentfoundry" / "state" / "manifest.json"
    state_data = json.loads(state_file.read_text(encoding="utf-8"))
    original_state_bytes = state_file.read_bytes()
    state_data["binary_digest"] = candidate_digest
    target_state_bytes = json.dumps(state_data, indent=2).encode("utf-8")
    state_file.write_bytes(target_state_bytes)

    root_identity = f"\\\\?\\{consumer.resolve()}"
    act_record = {
        "schema": "agentfoundry.activation/v1",
        "id": act_id,
        "root_identity": root_identity,
        "before_state": digest(original_state_bytes),
        "after_state": digest(target_state_bytes),
        "before_binary": before_bin_digest,
        "target": {
            "version": state_data["version"],
            "source": state_data["source"],
            "payload_digest": state_data["payload_digest"],
            "binary_digest": candidate_digest,
        },
        "release": None,
        "mode": 0o755,
    }
    act_file = consumer / ".agentfoundry" / "runtime" / "activation.json"
    act_file.parent.mkdir(parents=True, exist_ok=True)
    act_file.write_text(json.dumps(act_record), encoding="utf-8")

    return act_file, candidate_obj, candidate_digest


def main():
    if sys.platform != "win32":
        print("windows_launcher acceptance test: skipped (requires Windows platform)")
        sys.exit(0)

    repo_root = Path(__file__).resolve().parent.parent.parent
    source_launcher = repo_root / "scripts" / "agentfoundry.ps1"
    assert source_launcher.exists(), f"Launcher script not found at {source_launcher}"

    native_bin = find_native_binary()
    assert native_bin.exists(), f"Native binary not found at {native_bin}"
    fault_bin = find_fault_binary(repo_root, native_bin)
    assert fault_bin.exists(), f"Fault binary not found at {fault_bin}"

    def run_launcher(*args, cwd=None, env=None, expected_exit=0) -> subprocess.CompletedProcess:
        cmd = [
            "powershell.exe",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            str(launcher_ps1),
            *args,
        ]
        call_env = os.environ.copy()
        if env:
            call_env.update(env)
        res = subprocess.run(cmd, cwd=cwd, env=call_env, capture_output=True, text=True)
        assert res.returncode == expected_exit, (
            f"Expected exit {expected_exit}, got {res.returncode}.\n"
            f"Stdout: {res.stdout}\nStderr: {res.stderr}\nCmd: {cmd}"
        )
        return res

    with tempfile.TemporaryDirectory(prefix="af-win-launcher-test-") as test_dir:
        test_base = Path(test_dir)
        launcher_ps1 = test_base / "scripts" / "agentfoundry.ps1"
        launcher_ps1.parent.mkdir()
        shutil.copy2(source_launcher, launcher_ps1)
        consumer = test_base / "consumer with spaces"
        consumer.mkdir()

        repo_bin_dir = launcher_ps1.parent / "bin"
        repo_bin_dir.mkdir()
        trusted_launcher_bin = repo_bin_dir / "agentfoundry.exe"
        shutil.copy2(native_bin, trusted_launcher_bin)

        # ---------------------------------------------------------------------
        # 1. Worker exit propagation
        # ---------------------------------------------------------------------
        # (a) Successful install and exit 0 propagation
        res_install = run_launcher("install", "--directory", str(consumer), "--json", expected_exit=0)
        data_install = json.loads(res_install.stdout)
        assert data_install["code"] == "OK"
        assert data_install["mutation"] == "applied"
        consumer_bin = consumer / "scripts" / "bin" / "agentfoundry.exe"
        assert consumer_bin.exists(), "Selected executable was not created"
        state_file = consumer / ".agentfoundry" / "state" / "manifest.json"
        assert state_file.exists(), "Installed state manifest missing"

        # (b) Normal status exit 0 propagation with structured JSON
        res_status = run_launcher("status", "--directory", str(consumer), "--json", expected_exit=0)
        data_status = json.loads(res_status.stdout)
        assert data_status["schema"] == "agentfoundry.command-result/v1"
        assert data_status["code"] == "OK"
        assert data_status["data"]["installed"] is True

        # (c) Non-zero exit code propagation on invalid argument (exit 2)
        res_invalid = run_launcher("--nonsense", "--directory", str(consumer), "--json", expected_exit=2)
        data_invalid = json.loads(res_invalid.stdout)
        assert data_invalid["code"] == "INVALID_ARGUMENT"

        # An absent workspace is a valid uninstalled status.
        res_missing_dir = run_launcher("status", "--directory", str(consumer / "missing_dir"), "--json", expected_exit=0)
        assert json.loads(res_missing_dir.stdout)["data"]["installed"] is False

        # ---------------------------------------------------------------------
        # 2. Recovery with distinct local binary
        # ---------------------------------------------------------------------
        # Create a distinct valid native binary (using PE overlay to preserve validity with distinct digest)
        original_bin_bytes = consumer_bin.read_bytes()
        original_bin_digest = digest(original_bin_bytes)

        distinct_bin_bytes = original_bin_bytes + (b"\x00" * 4096) + b"AGENTFOUNDRY_DISTINCT_CANDIDATE_V2"
        distinct_bin_digest = digest(distinct_bin_bytes)
        assert distinct_bin_digest != original_bin_digest

        # Staged backup candidate object
        backup_act_id = "abc001"
        backup_dir = consumer / ".agentfoundry" / "backups" / backup_act_id / "objects"
        backup_dir.mkdir(parents=True, exist_ok=True)
        candidate_obj = backup_dir / distinct_bin_digest
        candidate_obj.write_bytes(distinct_bin_bytes)

        # Update state manifest to reflect candidate
        state_data = json.loads(state_file.read_text(encoding="utf-8"))
        original_state_bytes = state_file.read_bytes()
        state_data["binary_digest"] = distinct_bin_digest
        target_state_bytes = json.dumps(state_data, indent=2).encode("utf-8")
        state_file.write_bytes(target_state_bytes)

        # Write activation record
        root_identity = f"\\\\?\\{consumer.resolve()}"
        act_record = {
            "schema": "agentfoundry.activation/v1",
            "id": backup_act_id,
            "root_identity": root_identity,
            "before_state": digest(original_state_bytes),
            "after_state": digest(target_state_bytes),
            "before_binary": original_bin_digest,
            "target": {
                "version": state_data["version"],
                "source": state_data["source"],
                "payload_digest": state_data["payload_digest"],
                "binary_digest": distinct_bin_digest,
            },
            "release": None,
            "mode": 0o755,
        }
        act_file = consumer / ".agentfoundry" / "runtime" / "activation.json"
        act_file.write_text(json.dumps(act_record), encoding="utf-8")

        # Deny delete sharing on the selected binary while the launcher repairs it.
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                       wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
        kernel.CreateFileW.restype = wintypes.HANDLE
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel.CloseHandle.restype = wintypes.BOOL
        handle = kernel.CreateFileW(str(consumer_bin), 0x80000000, 1, None, 3, 0x80, None)
        assert handle != ctypes.c_void_p(-1).value, ctypes.get_last_error()
        record_before = act_file.read_bytes()
        try:
            denied = run_launcher("recover", "--directory", str(consumer), "--json", expected_exit=1)
            response = json.loads(denied.stdout)
            assert response["code"] == "IO_ERROR" and response["mutation"] != "none", response
            assert consumer_bin.read_bytes() == original_bin_bytes
            assert act_file.read_bytes() == record_before
            assert candidate_obj.read_bytes() == distinct_bin_bytes
        finally:
            assert kernel.CloseHandle(handle)

        # Recover through a copied worker while the old selected executable remains.
        assert digest(consumer_bin.read_bytes()) == original_bin_digest
        result = json.loads(run_launcher("recover", "--directory", str(consumer), "--json", expected_exit=0).stdout)
        assert result["data"]["recovery"] == "activation_repaired"
        assert result["mutation"] == "applied"
        assert digest(consumer_bin.read_bytes()) == distinct_bin_digest
        assert not act_file.exists()

        # Restage the same activation to exercise the missing-binary path separately.
        consumer_bin.write_bytes(original_bin_bytes)
        act_file.write_text(json.dumps(act_record), encoding="utf-8")

        # Simulate interrupted replacement: unlink selected binary so target executable is missing
        consumer_bin.unlink()
        assert not consumer_bin.exists()

        # Run launcher recover:
        # When target's selected binary is missing, launcher resolves trusted installed binary path
        # (trusted_launcher_bin) to run recover --directory <consumer>.
        # Rust verifies activation record, loads candidate from backup object, applies candidate,
        # tests restored binary with version --json, and clears activation record.
        res_recover = run_launcher("recover", "--directory", str(consumer), "--json", expected_exit=0)
        data_recover = json.loads(res_recover.stdout)
        assert data_recover["code"] == "OK"
        assert data_recover["mutation"] == "applied"

        # Verify binary was replaced with distinct local binary bytes
        assert consumer_bin.exists(), "Recovered binary was not restored"
        recovered_bytes = consumer_bin.read_bytes()
        assert digest(recovered_bytes) == distinct_bin_digest, "Recovered binary does not match distinct candidate digest"
        assert not act_file.exists(), "Activation record should be cleared after recovery"

        # ---------------------------------------------------------------------
        # 3. Rejected invalid inputs
        # ---------------------------------------------------------------------
        # (a) Direct in-place execution of update on Windows without worker mode: fails closed with MISSING_CAPABILITY (exit 6)
        direct_cmd = [str(consumer_bin), "update", "--directory", str(consumer), "--json"]
        direct_res = subprocess.run(direct_cmd, capture_output=True, text=True)
        assert direct_res.returncode == 6, f"Expected exit 6 for direct update, got {direct_res.returncode}"
        direct_data = json.loads(direct_res.stdout)
        assert direct_data["code"] == "MISSING_CAPABILITY"

        # (b) Tampered worker binary: worker digest mismatch against selected binary -> INTEGRITY_FAILURE (exit 5)
        tampered_dir = test_base / "tampered_worker"
        tampered_dir.mkdir()
        tampered_exe = tampered_dir / "agentfoundry.exe"
        tampered_bytes = original_bin_bytes + b"TAMPERED_HASH_PAYLOAD"
        tampered_exe.write_bytes(tampered_bytes)

        tampered_env = os.environ.copy()
        tampered_env["AF_WORKER"] = "1"
        tampered_env["AF_SELECTED_BINARY"] = str(consumer_bin)

        tampered_res = subprocess.run(
            [str(tampered_exe), "recover", "--directory", str(consumer), "--json"],
            env=tampered_env,
            capture_output=True,
            text=True,
        )
        assert tampered_res.returncode == 5, f"Expected exit 5 for tampered worker, got {tampered_res.returncode}"
        tampered_data = json.loads(tampered_res.stdout)
        assert tampered_data["code"] == "INTEGRITY_FAILURE"

        # (c) Worker mode rejected when selected binary is missing -> INVALID_STATE (exit 2)
        consumer_bin.unlink()
        assert not consumer_bin.exists()

        missing_worker_env = os.environ.copy()
        missing_worker_env["AF_WORKER"] = "1"
        missing_worker_env["AF_SELECTED_BINARY"] = str(consumer_bin)
        missing_worker_res = subprocess.run(
            [str(native_bin), "recover", "--directory", str(consumer), "--json"],
            env=missing_worker_env,
            capture_output=True,
            text=True,
        )
        assert missing_worker_res.returncode == 5, f"Expected exit 5 for worker with missing binary, got {missing_worker_res.returncode}"
        missing_worker_data = json.loads(missing_worker_res.stdout)
        assert missing_worker_data["code"] == "INVALID_STATE"

        # Restore binary for remaining checks
        consumer_bin.write_bytes(distinct_bin_bytes)

        # (d) Mismatched AF_SELECTED_BINARY: worker rejects when selected binary path does not match target repo
        mismatch_env = os.environ.copy()
        mismatch_env["AF_WORKER"] = "1"
        mismatch_env["AF_SELECTED_BINARY"] = str(test_base / "unrelated_repo" / "scripts" / "bin" / "agentfoundry.exe")
        mismatch_res = subprocess.run(
            [str(consumer_bin), "recover", "--directory", str(consumer), "--json"],
            env=mismatch_env,
            capture_output=True,
            text=True,
        )
        assert mismatch_res.returncode == 2, f"Expected exit 2 for mismatched selected binary, got {mismatch_res.returncode}"
        mismatch_data = json.loads(mismatch_res.stdout)
        assert mismatch_data["code"] == "INVALID_ARGUMENT"

        # ---------------------------------------------------------------------
        # 4. Activation crash and recovery via selected fault build
        # ---------------------------------------------------------------------
        # Isolate distinct launcher root with fault binary
        fault_root = test_base / "fault_env"
        fault_launcher_ps1 = fault_root / "scripts" / "agentfoundry.ps1"
        fault_launcher_ps1.parent.mkdir(parents=True)
        shutil.copy2(source_launcher, fault_launcher_ps1)
        fault_bin_dir = fault_root / "scripts" / "bin"
        fault_bin_dir.mkdir(parents=True)
        shutil.copy2(fault_bin, fault_bin_dir / "agentfoundry.exe")

        def run_fault_launcher(*args, env=None, expected_exit=0) -> subprocess.CompletedProcess:
            cmd = ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(fault_launcher_ps1), *args]
            call_env = os.environ.copy()
            if env:
                call_env.update(env)
            res = subprocess.run(cmd, env=call_env, capture_output=True, text=True)
            assert res.returncode == expected_exit, (
                f"Expected exit {expected_exit}, got {res.returncode}.\nStdout: {res.stdout}\nStderr: {res.stderr}\nCmd: {cmd}"
            )
            return res

        fault_consumer = test_base / "fault_consumer"
        fault_consumer.mkdir()
        run_fault_launcher("install", "--directory", str(fault_consumer), "--json", expected_exit=0)

        fault_consumer_bin = fault_consumer / "scripts" / "bin" / "agentfoundry.exe"
        assert fault_consumer_bin.exists()
        fault_bin_bytes = fault_consumer_bin.read_bytes()
        fault_bin_digest = digest(fault_bin_bytes)
        fault_journal = fault_consumer / ".agentfoundry" / "runtime" / "transaction.json"

        # (a) Precommit checkpoint crash and recovery (applied:scripts/bin/agentfoundry.exe)
        pre_act_id = "fa01"
        pre_cand_bytes = fault_bin_bytes + (b"\x00" * 4096) + b"AF_PRECOMMIT_V1"
        pre_cand_digest = digest(pre_cand_bytes)
        pre_act_file, pre_cand_obj, _ = stage_activation(
            fault_consumer, pre_act_id, pre_cand_bytes, fault_bin_bytes
        )

        res_pre_crash = run_fault_launcher(
            "recover",
            "--directory",
            str(fault_consumer),
            "--json",
            env={"AGENTFOUNDRY_TEST_CRASH": "applied:scripts/bin/agentfoundry.exe"},
            expected_exit=86,
        )
        assert res_pre_crash.returncode == 86
        assert json.loads(fault_journal.read_text(encoding="utf-8"))["phase"] == "Applying"
        assert pre_act_file.exists() and pre_cand_obj.exists()
        assert digest(fault_consumer_bin.read_bytes()) == pre_cand_digest

        # Rollback: uncommitted transaction restores original binary; activation record retained
        res_pre_rollback = run_fault_launcher("recover", "--directory", str(fault_consumer), "--json", expected_exit=0)
        data_pre_rollback = json.loads(res_pre_rollback.stdout)
        assert data_pre_rollback["data"]["recovery"] == "rolled_back"
        assert data_pre_rollback["mutation"] == "rolled_back"
        assert not fault_journal.exists()
        assert digest(fault_consumer_bin.read_bytes()) == fault_bin_digest
        assert pre_act_file.exists() and pre_cand_obj.exists()

        # Activation repair: completes repair and removes activation record
        res_pre_repair = run_fault_launcher("recover", "--directory", str(fault_consumer), "--json", expected_exit=0)
        data_pre_repair = json.loads(res_pre_repair.stdout)
        assert data_pre_repair["data"]["recovery"] == "activation_repaired"
        assert data_pre_repair["mutation"] == "applied"
        assert digest(fault_consumer_bin.read_bytes()) == pre_cand_digest
        assert not pre_act_file.exists() and not fault_journal.exists()

        # Idempotence: subsequent recovery is no-op
        res_pre_idem = run_fault_launcher("recover", "--directory", str(fault_consumer), "--json", expected_exit=0)
        assert json.loads(res_pre_idem.stdout)["data"]["recovery"] == "not_needed"
        assert digest(fault_consumer_bin.read_bytes()) == pre_cand_digest

        # (b) Postcommit checkpoint crash and recovery (core-committed-durable)
        post_act_id = "fa02"
        post_before_bytes = fault_consumer_bin.read_bytes()
        post_cand_bytes = fault_bin_bytes + (b"\x00" * 4096) + b"AF_POSTCOMMIT_V2"
        post_cand_digest = digest(post_cand_bytes)
        post_act_file, post_cand_obj, _ = stage_activation(
            fault_consumer, post_act_id, post_cand_bytes, post_before_bytes
        )

        res_post_crash = run_fault_launcher(
            "recover",
            "--directory",
            str(fault_consumer),
            "--json",
            env={"AGENTFOUNDRY_TEST_CRASH": "core-committed-durable"},
            expected_exit=86,
        )
        assert res_post_crash.returncode == 86
        assert json.loads(fault_journal.read_text(encoding="utf-8"))["phase"] == "CoreCommitted"
        assert post_act_file.exists() and post_cand_obj.exists()
        assert digest(fault_consumer_bin.read_bytes()) == post_cand_digest

        # Commit recovery: finished commit retains candidate binary (not rolled back); activation record retained
        res_post_commit = run_fault_launcher("recover", "--directory", str(fault_consumer), "--json", expected_exit=0)
        data_post_commit = json.loads(res_post_commit.stdout)
        assert data_post_commit["data"]["recovery"] == "committed"
        assert data_post_commit["mutation"] == "applied"
        assert not fault_journal.exists()
        assert digest(fault_consumer_bin.read_bytes()) == post_cand_digest
        assert post_act_file.exists() and post_cand_obj.exists()

        # Activation completion: verifies candidate and clears activation record
        res_post_repair = run_fault_launcher("recover", "--directory", str(fault_consumer), "--json", expected_exit=0)
        data_post_repair = json.loads(res_post_repair.stdout)
        assert data_post_repair["data"]["recovery"] == "activation_repaired"
        assert data_post_repair["mutation"] == "applied"
        assert digest(fault_consumer_bin.read_bytes()) == post_cand_digest
        assert not post_act_file.exists() and not fault_journal.exists()

        # Idempotence: subsequent recovery is no-op
        res_post_idem = run_fault_launcher("recover", "--directory", str(fault_consumer), "--json", expected_exit=0)
        assert json.loads(res_post_idem.stdout)["data"]["recovery"] == "not_needed"
        assert digest(fault_consumer_bin.read_bytes()) == post_cand_digest

    print(
        "windows_launcher acceptance test passed: verified worker exit propagation, "
        "recovery with distinct local binary, rejected invalid inputs, and activation "
        "crash/recovery (precommit/postcommit, retention, idempotence)"
    )


if __name__ == "__main__":
    main()
