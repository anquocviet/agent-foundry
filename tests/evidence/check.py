#!/usr/bin/env python3
"""Exercise installed onboarding tools without manufacturing documentation."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
CAPSULE_BEGIN = "<!-- ONBOARDING_EVIDENCE_CAPSULE_V2:BEGIN -->"
CAPSULE_END = "<!-- ONBOARDING_EVIDENCE_CAPSULE_V2:END -->"
BUNDLE_END = "<!-- ONBOARDING_EVIDENCE_BUNDLE_V2:END -->"


class OnboardingEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="harness-onboarding-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.run_command(
            [ROOT / "target/release/agentfoundry", "install", "--directory", self.root]
        )
        (self.root / "src").mkdir()
        (self.root / "src/status.sh").write_text("#!/bin/sh\nprintf 'ready\\n'\n")
        self.run_command(["git", "init", "--quiet", "--initial-branch=test"])
        self.run_command(["git", "add", "."])
        self.run_command([
            "git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.test",
            "commit", "--quiet", "-m", "Pin existing repository knowledge",
        ])
        self.revision = self.run_command(["git", "rev-parse", "HEAD"]).stdout.strip()
        self.emitter = self.root / ".agents/skills/agentfoundry-onboard/scripts/emit_evidence_bundle.py"
        self.validator = self.root / ".agents/skills/agentfoundry-audit/scripts/validate_evidence_capsule.py"
        self.before = self.snapshot()
        digest = hashlib.sha256(b"fixture boundary observation").hexdigest()
        self.spec = {
            "boundary": [
                {"id": f"B{index}", "kind": kind, "initial_evidence_sha256": digest,
                 "final_evidence_sha256": digest, "result": "Pass", "notes": []}
                for index, kind in enumerate(
                    ("git", "ignored_or_managed", "runtime", "temporary_paths"), 1
                )
            ],
            "claims": [],
            "hunks": [],
            "limitations": ["No backfill is needed for the inspected path."],
        }

    def run_command(self, command, text=None, expected=0):
        environment = os.environ.copy()
        environment.pop("PYTHONDONTWRITEBYTECODE", None)
        result = subprocess.run(
            command, cwd=self.root, input=text, capture_output=True, text=True,
            check=False, env=environment,
        )
        self.assertEqual(result.returncode, expected, result.stderr)
        return result

    def snapshot(self):
        return {
            "files": {
                str(path.relative_to(self.root)): path.read_bytes()
                for path in self.root.rglob("*") if path.is_file()
            },
            "directories": {
                str(path.relative_to(self.root))
                for path in self.root.rglob("*") if path.is_dir()
            },
        }

    def emit(self, expected=0):
        return self.run_command([
            sys.executable, self.emitter, "--repository", self.root,
            "--revision", self.revision, "--branch", "test",
        ], json.dumps(self.spec), expected)

    def validate(self, bundle, expected=0):
        return self.run_command([
            sys.executable, self.validator, "--repository", self.root,
        ], bundle, expected)

    def validate_transcript(self, bundle, expected=0, digest_override=None):
        events = [
            {"type": "response_item", "payload": {
                "type": "custom_tool_call_output", "output": bundle,
            }},
            {"type": "event_msg", "payload": {
                "type": "task_complete", "last_agent_message": "No backfill proposed.",
            }},
        ]
        raw = "".join(json.dumps(event) + "\n" for event in events).encode()
        digest = hashlib.sha256(raw).hexdigest()
        with tempfile.TemporaryDirectory(prefix="harness-transcript-") as directory:
            transcript = Path(directory) / "session.jsonl"
            transcript.write_bytes(raw)
            result = self.run_command([
                sys.executable, self.validator, "--repository", self.root,
                "--transcript", transcript, "--expected-transcript-sha256",
                digest_override or digest,
            ], expected=expected)
        return result, digest

    def capsule(self, bundle):
        return json.loads(bundle.split(CAPSULE_BEGIN + "\n```json\n", 1)[1]
                          .split("\n```\n" + CAPSULE_END, 1)[0])

    def bundle(self, capsule, prefix=""):
        inner = prefix + CAPSULE_BEGIN + "\n```json\n" + json.dumps(capsule)
        inner += "\n```\n" + CAPSULE_END + "\n"
        digest = hashlib.sha256(inner.encode()).hexdigest()
        return f"<!-- ONBOARDING_EVIDENCE_BUNDLE_V2:BEGIN sha256={digest} -->\n{inner}{BUNDLE_END}\n"

    def test_no_backfill_roundtrip_creates_no_files(self):
        bundle = self.emit().stdout
        self.assertNotIn("<!-- ONBOARDING_PATCH:", bundle)
        result = json.loads(self.validate(bundle).stdout)
        self.assertTrue(result["valid"])
        self.assertEqual(result["hunk_count"], 0)
        self.assertEqual(result["claim_count"], 0)
        self.assertEqual(result["patch_sha256"], {})
        self.assertEqual(self.snapshot(), self.before)

    def add_proposal(self):
        self.spec["claims"] = [{
            "id": "C1", "hunk_id": "H1", "text": "Task status is implemented in src/status.sh.",
            "classification": "Observed", "sources": [{
                "path": "src/status.sh", "start_line": 1, "end_line": 2,
                "role": "implementation",
            }],
        }]
        self.spec["hunks"] = [{
            "id": "H1", "destination": "AGENTS.md", "unknowns": [],
            "after_text": (self.root / "AGENTS.md").read_text()
                          + "\nSee [task status](src/status.sh).\n",
        }]

    def test_existing_proposal_with_code_evidence_still_passes(self):
        self.add_proposal()
        bundle = self.emit().stdout
        result = json.loads(self.validate(bundle).stdout)
        self.assertEqual(result["hunk_count"], 1)
        source = self.capsule(bundle)["claims"][0]["sources"][0]
        self.assertEqual(source["content_sha256"], hashlib.sha256(
            (self.root / "src/status.sh").read_bytes()).hexdigest())
        self.assertEqual(self.snapshot(), self.before)

    def test_no_backfill_still_requires_complete_boundary(self):
        capsule = self.capsule(self.emit().stdout)
        self.spec["boundary"].pop()
        self.assertIn("boundary must cover", self.emit(expected=1).stderr)
        capsule["boundary"].pop()
        self.assertIn("boundary must cover", self.validate(
            self.bundle(capsule), expected=1).stderr)

    def test_no_backfill_boundary_hash_invariants_still_apply(self):
        capsule = self.capsule(self.emit().stdout)
        self.spec["boundary"][0]["final_evidence_sha256"] = "0" * 64
        self.assertIn("Pass requires equal evidence hashes", self.emit(expected=1).stderr)
        capsule["boundary"][0]["final_evidence_sha256"] = "0" * 64
        self.assertIn("Pass requires equal initial/final evidence hashes", self.validate(
            self.bundle(capsule), expected=1).stderr)
        for row in (self.spec["boundary"][0], capsule["boundary"][0]):
            row["final_evidence_sha256"] = row["initial_evidence_sha256"]
            row["result"] = "Fail"
        self.assertIn("Fail requires different evidence hashes", self.emit(expected=1).stderr)
        self.assertIn("Fail requires different initial/final evidence hashes", self.validate(
            self.bundle(capsule), expected=1).stderr)

    def test_no_backfill_unknown_boundary_is_preserved(self):
        row = self.spec["boundary"][2]
        row.update(result="Unknown", initial_evidence_sha256=None,
                   final_evidence_sha256=None, notes=["Runtime was not observed."])
        bundle = self.emit().stdout
        self.validate(bundle)
        self.assertEqual(self.capsule(bundle)["boundary"][2], row)

    def test_no_backfill_transcript_authentication_still_applies(self):
        bundle = self.emit().stdout
        response, digest = self.validate_transcript(bundle)
        result = json.loads(response.stdout)
        self.assertEqual(result["transcript_sha256"], digest)
        self.assertEqual(result["evidence_source"], "machine_tool_output")
        self.assertEqual(result["hunk_count"], 0)
        response, _ = self.validate_transcript(bundle, expected=1, digest_override="0" * 64)
        self.assertIn("transcript SHA-256 mismatch", response.stderr)
        self.assertEqual(self.snapshot(), self.before)

    def test_no_backfill_requires_the_pinned_repository_root(self):
        capsule = self.capsule(self.emit().stdout)
        capsule["tested_repository"]["root"] = str(self.root / "another-repository")
        self.assertIn("--repository must match tested_repository.root", self.validate(
            self.bundle(capsule), expected=1).stderr)

    def test_no_backfill_still_authenticates_producer(self):
        capsule = self.capsule(self.emit().stdout)
        capsule["producer_skill"]["sha256"] = "0" * 64
        self.assertIn("producer_skill.sha256 does not match", self.validate(
            self.bundle(capsule), expected=1).stderr)

    def test_no_backfill_cannot_hide_a_patch(self):
        capsule = self.capsule(self.emit().stdout)
        patch = "<!-- ONBOARDING_PATCH:H1:BEGIN -->\n```diff\n-hidden\n+change\n```\n<!-- ONBOARDING_PATCH:H1:END -->\n"
        self.assertIn("no-backfill capsule must not contain patch markers", self.validate(
            self.bundle(capsule, patch), expected=1).stderr)

    def test_orphan_claims_and_hunks_remain_invalid(self):
        self.add_proposal()
        capsule = self.capsule(self.emit().stdout)
        hunks = self.spec["hunks"]
        self.spec["hunks"] = []
        self.assertIn("every claim hunk_id", self.emit(expected=1).stderr)
        capsule_without_hunks = dict(capsule, hunks=[])
        self.assertIn("every claim must be referenced", self.validate(
            self.bundle(capsule_without_hunks), expected=1).stderr)
        self.spec["hunks"] = hunks
        self.spec["claims"] = []
        self.assertIn("no claims reference this hunk", self.emit(expected=1).stderr)
        capsule_without_claims = dict(capsule, claims=[])
        self.assertIn("unknown claim C1", self.validate(
            self.bundle(capsule_without_claims), expected=1).stderr)

    def test_code_source_hash_mismatch_still_fails(self):
        self.add_proposal()
        capsule = self.capsule(self.emit().stdout)
        capsule["claims"][0]["sources"][0]["content_sha256"] = "0" * 64
        self.assertIn("content_sha256 does not match pinned source bytes", self.validate(
            self.bundle(capsule), expected=1).stderr)

    def test_no_backfill_bundle_digest_mismatch_fails(self):
        bundle = self.emit().stdout.replace("No backfill", "Changed backfill")
        self.assertIn("machine bundle SHA-256 does not match", self.validate(
            bundle, expected=1).stderr)

    def test_legacy_v1_still_requires_a_proposal(self):
        capsule = self.capsule(self.emit().stdout)
        capsule["schema"] = "onboarding-evidence-capsule/v1"
        message = CAPSULE_BEGIN.replace("_V2", "_V1") + "\n```json\n"
        message += json.dumps(capsule) + "\n```\n" + CAPSULE_END.replace("_V2", "_V1")
        self.assertIn("v1 claims must be a non-empty array", self.validate(
            message, expected=1).stderr)

    def test_legacy_v1_patch_accepts_only_legacy_guarantees(self):
        self.add_proposal()
        bundle = self.emit().stdout
        capsule = self.capsule(bundle)
        capsule['schema'] = 'onboarding-evidence-capsule/v1'
        hunk = capsule['hunks'][0]
        hunk['boundary'] = 'Complete AGENTS.md file'
        hunk['before_sha256'] = hunk.pop('destination_before_sha256')
        hunk['after_sha256'] = hunk.pop('destination_after_sha256')
        # Legacy validation checks format and patch digest, not pinned source truth.
        capsule['claims'][0]['sources'][0]['content_sha256'] = '0' * 64
        prefix = bundle.split(CAPSULE_BEGIN, 1)[0].split('\n', 1)[1]
        message = prefix + CAPSULE_BEGIN.replace('_V2', '_V1') + '\n```json\n'
        message += json.dumps(capsule) + '\n```\n' + CAPSULE_END.replace('_V2', '_V1')
        result = json.loads(self.validate(message).stdout)
        self.assertTrue(result['valid'])
        self.assertEqual(result['schema'], 'onboarding-evidence-capsule/v1')
        self.assertEqual(result['patch_sha256'], {'H1': hunk['patch_sha256']})
        self.assertEqual(self.snapshot(), self.before)


    def assert_explicit_skill_policy(self, metadata, skill):
        import re
        policies = re.findall(r"^\s*allow_implicit_invocation:\s*(\S+)", metadata, re.M)
        self.assertEqual(policies, ["false"], "skill must require explicit user invocation")
        description = next((line for line in skill.splitlines() if line.startswith("description:")), "")
        self.assertIn("explicit", description, "description must require explicit invocation")

    def test_installed_core_skills_require_explicit_invocation(self):
        skills = self.root / ".agents/skills"
        self.assertEqual({path.name for path in skills.iterdir()}, {
            "agentfoundry-onboard", "agentfoundry-audit",
            "agentfoundry-improve", "agentfoundry-encode-invariant",
        })
        for skill in skills.iterdir():
            with self.subTest(skill=skill.name):
                self.assert_explicit_skill_policy(
                    (skill / "agents/openai.yaml").read_text(),
                    (skill / "SKILL.md").read_text(),
                )

    def test_implicit_invocation_policy_is_rejected(self):
        metadata = "policy:\n  allow_implicit_invocation: true\n"
        skill = "Use only when the user explicitly invokes `$example`."
        with self.assertRaisesRegex(AssertionError, "require explicit user invocation"):
            self.assert_explicit_skill_policy(metadata, skill)

    def test_body_instruction_does_not_replace_explicit_description(self):
        metadata = "policy:\n  allow_implicit_invocation: false\n"
        skill = ("---\ndescription: Use when reviewing code.\n---\n"
                 "Use only when the user explicitly invokes `$example`.")
        with self.assertRaisesRegex(AssertionError, "description must require"):
            self.assert_explicit_skill_policy(metadata, skill)

    def test_installed_context_has_no_task_state(self):
        # Consumer task data is outside AgentFoundry's managed payload contract.
        for target in ("harness.db", "agentfoundry.db", ".agentfoundry/tasks.db"):
            self.assertFalse((self.root / target).exists())
        active = self.root / "docs/agentfoundry/plans/active"
        self.assertEqual({path.name for path in active.iterdir()}, {"README.md"})

    def test_oversized_spec_and_invalid_encoding_fail_without_writes(self):
        too_large = " " * (16 * 1024 * 1024 + 1)
        result = self.run_command([
            sys.executable, self.emitter, "--repository", self.root,
            "--revision", self.revision, "--branch", "test",
        ], too_large, expected=1)
        self.assertIn("INPUT_LIMIT_EXCEEDED", result.stderr)
        result = subprocess.run([sys.executable, str(self.validator), "--repository", str(self.root)],
                                input=b"\xff", capture_output=True, cwd=self.root)
        self.assertEqual(result.returncode, 1)
        self.assertFalse(json.loads(result.stderr)["valid"])
        self.assertEqual(self.snapshot(), self.before)

    def test_portable_source_path_escapes_fail_without_writes(self):
        self.add_proposal()
        for path in ('C:/outside', '..\\outside', 'src//status.sh', 'src/./status.sh'):
            with self.subTest(path=path):
                self.spec['claims'][0]['sources'][0]['path'] = path
                result = self.emit(expected=1)
                self.assertTrue('path' in result.stderr)
        self.assertEqual(self.snapshot(), self.before)

    def test_git_output_limit_rejects_incomplete_evidence_without_writes(self):
        helper = self.emitter.parent
        script = ('import sys; sys.dont_write_bytecode = True; '
                  'sys.path.insert(0, sys.argv[1]); from git_readonly import git_read; '
                  'git_read(sys.argv[2], "show", sys.argv[3] + ":src/status.sh", limit=4)')
        result = self.run_command([sys.executable, '-c', script, str(helper), str(self.root), self.revision], expected=1)
        self.assertIn('INPUT_LIMIT_EXCEEDED', result.stderr)
        self.assertEqual(self.snapshot(), self.before)



if __name__ == "__main__":
    unittest.main()
