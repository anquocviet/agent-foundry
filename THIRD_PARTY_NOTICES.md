# Third-party notices

repository-harness, https://github.com/hoangnb24/repository-harness, commit `485fd4062be1c71b27c1c9e5585ac4cd2faa7e06`.

Reused payload files (modified skill namespaces, entrypoint markers and docs links; evidence schema names/hash semantics unchanged):

- `.agents/skills/audit-onboarding-proposal/SKILL.md`
- `.agents/skills/audit-onboarding-proposal/agents/openai.yaml`
- `.agents/skills/audit-onboarding-proposal/scripts/validate_evidence_capsule.py`
- `.agents/skills/encode-invariant/SKILL.md`
- `.agents/skills/encode-invariant/agents/openai.yaml`
- `.agents/skills/improve-harness/SKILL.md`
- `.agents/skills/improve-harness/agents/openai.yaml`
- `.agents/skills/onboard-repository/SKILL.md`
- `.agents/skills/onboard-repository/agents/openai.yaml`
- `.agents/skills/onboard-repository/references/evidence-capsule-v1.md`
- `.agents/skills/onboard-repository/references/evidence-capsule-v2.md`
- `.agents/skills/onboard-repository/scripts/emit_evidence_bundle.py`
- `.agents/skills/onboard-repository/scripts/render_patch.py`
- `scripts/agent-harness-block.md`
- `docs/WORKFLOW.md`
- `docs/README.md`
- `docs/patterns/encoding-invariants.md`
- `docs/product/README.md`
- `docs/plans/README.md`
- `docs/plans/active/README.md`
- `docs/plans/completed/README.md`
- `docs/decisions/README.md`
- `docs/templates/application-runbook.md`
- `docs/templates/decision.md`
- `docs/templates/exec-plan.md`
- `docs/templates/harness-improvement.md`
- `.agents/skills/engineering-wisdom/SKILL.md`
- `.agents/skills/engineering-wisdom/agents/openai.yaml`
- `.agents/skills/engineering-wisdom/references/heuristics.md`
- `.agents/skills/engineering-wisdom/references/sources.md`

MIT License

Copyright (c) 2025 Hoang Nguyen

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

Regression source: `tests/workflow/test-onboarding-evidence.py`, same pinned commit; binary and skill paths adapted in `tests/evidence/check.py`.

Additional modifications: evidence absolute-root validation uses native `pathlib.Path.is_absolute()` rather than POSIX string prefix; generic template/pattern/plans references point to `docs/agentfoundry/`. Native Windows execution remains unverified.

Evidence helper modifications additionally bound stdin specs (16 MiB), after-images (8 MiB), transcripts/messages (100 MiB), and return structured UTF-8 decoding errors. A new shared stdlib Git reader bounds each output pipe to 8 MiB and execution/capture to 30 seconds (worktree-root output: 64 KiB), disables optional Git locks, and rejects incomplete evidence. Portable evidence paths reject drive separators, backslashes and empty/dot components. Installed no-write/output-limit regressions and positive legacy-v1 compatibility are exercised in `tests/evidence/check.py`.

Migration compatibility registry in `crates/agentfoundry/src/migration.rs` records SHA-256 identities from that source baseline and upstream release `harness-v0.1.10`, commit `e765792b635b4d5e3e5fc0578f82f9ca5dea2681`. This adds compatibility recognition, not a change to the canonical payload baseline. `tests/acceptance/migration.py` reads explicitly supplied pinned Git objects to construct disposable legacy fixtures; it never runs the upstream installer.
