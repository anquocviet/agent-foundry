# Implementation status

Development build, not v1. Scope and completion gates remain proposal §24. Target: current workspace (actual directory `agent-foundary`); proposal unchanged. No pre-existing source or instructions found.

| Issue | Dependencies | Deliverables | Acceptance | Status |
|---|---|---|---|---|
| AF-001 — Baseline và scaffold | Không | Workspace, toolchain/lockfile, notices, ADR-001..012 drafts, parity matrix | Build CLI skeleton; source baseline pinned; phạm vi không bị đổi thành MVP | in_progress |
| AF-002 — Domain và schemas | AF-001 | Types cho path/hash/ownership/distribution/plan/state/journal/session; schemas | Positive/negative validation; duplicate/path escape/version mismatch reject | in_progress |
| AF-003 — Canonical payload | AF-001,02 | Manifest, protocol docs, template assets, namespaced skills skeleton | Exact manifest→embedded generation; no upstream app assumptions; installed links test | in_progress |
| AF-004 — Read-only inventory/status/doctor | AF-002,03 | Snapshot reader, safe parser, output envelope, diagnostics | Không mutation/network mặc định; local modifications không thành corruption | in_progress |
| AF-005 — Safe filesystem và lock | AF-002 | OS adapters, no-follow checks, lock, metadata preservation | T-039..041; path+sentinel tests Linux/Windows; giới hạn được ghi | in_progress |
| AF-006 — Transaction engine và recovery | AF-002,05 | Journal, backup, stage, checkpoint, rollback, public recover | T-029..036, UP-04 regression; recovery trước load active state | in_progress |
| AF-007 — Install và shared blocks | AF-003..06 | All install modes, idempotency, preview, shim integration | T-001..013; no whole docs move; every write qua engine | in_progress |
| AF-008 — Three-way planner | AF-002,04,05 | B/L/U rules, Git adapter, structured conflicts, deterministic plan | T-014..019; handling `1..127` conflicts và process errors | in_progress |
| AF-009 — Frozen plan và resolution | AF-006,08 | Snapshot revalidation, plan-out/apply-plan, stage/continue/abort | T-020..028; UP-03; pending session không bị xóa bởi normal update | in_progress |
| AF-010 — Release client và candidate | AF-002,05 | Metadata validation, downloader, identity, timeout, trust model | T-042..045,59; test overrides không có trong production | in_progress (pinned HTTPS observation and verified downloader/export implemented; positive published-release and native supervisor proof pending) |
| AF-011 — Self-update và activation repair | AF-006,09,10 | Public update supervisor, selected binary activation, Windows helper | T-031,37,38,44; core commit/binary failure explicit, không success giả | in_progress (clean updates retain independent activation record and offline repair; conflict-session ordering, positive published release, complete crash matrix and Windows helper remain open) |
| AF-012 — Bootstrap parity | AF-007,10,11 | Bash và PowerShell wrappers mỏng, interactive choices ở core | T-006..009,13; không script nào move docs/append state độc lập | not_started |
| AF-013 — Evidence emit/render/validate | AF-003,04 | Port/reuse helpers, v1 reader/v2 emitter, schemas/fixtures | T-050..053; no-backfill; pinned sources; strict no-write kể cả pycache | in_progress |
| AF-014 — Skills hoàn chỉnh và examples | AF-003,13 | 5 skill specs, client metadata, full/patch/reissue audit workflows | T-049,54,55; independent audit contract; no auto-orchestration | in_progress |
| AF-015 — Harness migration | AF-007..09,13 | Known-version detector, mapping/preview, migration transaction | T-056,57; giữ old state/legacy, shared markers chỉ thay explicit | in_progress (public inventory/preview/saved apply and wisdom/Claude proof; migration-specific crash/native coverage pending) |
| AF-016 — CI và multi-platform packaging | AF-005..14 | Native matrix, release pipeline, exact artifacts, proof report | T-058..060; build và smoke trên từng platform; source/pointer ordering | in_progress |
| AF-017 — Consumer docs và pilot | AF-007..16 | Quickstart, recovery guide, compatibility, examples, contributor/security docs | Người mới theo docs cài được; pilot có baseline/rerun; limits không bị giấu | in_progress |
| AF-018 — Release readiness review | Tất cả | Parity evidence hoàn chỉnh, risk review, release candidate | Mọi MUST pass hoặc release blocked; không tự publish khi thiếu authorization | not_started |

## Release blockers

All unverified PAR/T rows; macOS, Linux arm64 and Windows native runners; public release owner/trust endpoint; owner license approval before public release. No external settings changed; no publication authorized.

## Continuation checkpoint

See `docs/milestone-evidence.md`. AF-001/002 remain partial: architectural ports, complete schema/model integration and native toolchain proof are not done. Transaction journals/backups now use content-addressed images; large saved install plans remain inline and bounded. No dependency milestone is declared completed from partial code.

## Continued implementation checkpoint

Strict config/output schemas, pure snapshot/resolution/lifecycle models, release identity validators, internal object allowlist, mode drift checks and nested-repository refusal exist. Public recover is exercised by real process exits at pre-journal, durable journal, file apply, manifest commit, pre-completion and interrupted rollback boundaries. Crash hooks are feature-gated with distinct build identity; packaging rejects enabled hooks. Evidence Git reads now bound output/lifetime and reject portable path escapes without consumer writes. Next: complete snapshot reader and architectural ports, then public update sessions and candidate activation; large saved-plan staging remains open. Migration command remains unavailable: research confirmed upstream `.harness-core/manifest.json` schema 1 with `core_version` and `files[{path,upstream_sha256}]`, baselines under `.harness-core/base/`; do not equate self-declared version/hash with trusted baseline provenance.

Latest verified checkpoint: Rust 27 tests, installed evidence 21 tests, all-feature Clippy, schema checks and packaged public acceptance passed. Package SHA-256 `831f1937560319d606fe8e7c4b710535d4eadb757d25a96cce633f95c2618acc`; exact identities in milestone-evidence. Recovery now refuses external chmod drift, corrupt/missing backups and malformed/unknown-schema journals while retaining evidence. Public POSIX lock-contention proof passed. Status includes selections/generation/running identity; doctor rejects missing executable bits. Installed payload membership/bytes/links passed, including optional addon/Claude. Full AF-001/002 and release gates remain open; these checks do not certify the unimplemented updater, sessions, activation, migration or native platform matrix.

Saved-plan continuation: `--plan-out` now rejects serialized outputs above the apply reader's 16 MiB limit before file creation, creates Unix outputs with `0600`, and bounds actual apply reads. Packaged saved-plan roundtrip/private-mode check and oversized private-preimage refusal passed; external large-plan staging remains unimplemented. New package SHA-256 `732a09b166653a844d3f614953054aaabc2943c9b9d6ec5e6a6dd56693ca3f01`; source identity and commands in milestone evidence. Unmanaged docs adoption proof preserves consumer WORKFLOW/ARCHITECTURE and does not fabricate ownership.

Snapshot/validation continuation: `infrastructure::snapshot` and `verify_snapshot` populate/recheck the pure model; offline planning freezes managed containers, baseline blobs, config, manifest and executable, then verifies snapshot and plan preimages. `tests/snapshot.rs` covers preserved-file/absent-path/config/mode drift. Distribution, installed-state, snapshot and transaction path sets reject file/parent collisions. AF-002/004/009 remain partial: no persisted conflict session, complete typed schema integration or cross-release supervisor. Latest package identity and full-gate outcome are recorded in milestone evidence; earlier hashes above are historical, not current-source proof.

AF-002 continuation: snapshot/session DTOs now have strict Serde wire support, bounded semantic parse entrypoints, required nullable fields, explicit snapshot schema and state/config entry fingerprint checks. Conflict LOCAL must match the frozen entry. Portable paths validate while deserializing. AF-002 remains open for other complete proposal model contracts; AF-009 public persistence/continue/abort remains open. See latest milestone evidence rather than prior package identities.

Saved offline update plans now use the shared filesystem plan adapter: `update --offline --dry-run --plan-out` and `--apply-plan` verify preimages before exact authorization against the embedded installed build. Public scenarios cover no-op roundtrip and edited/missing preserved input refusal. Entry ownership uses the existing typed enum end-to-end; snapshot generation binds to parsed manifest bytes and Unix roots bind device/inode plus canonical path. All broader AF gates remain open as listed; these additions do not complete sessions/upgrades/activation.

Migration continuation: public `migrate harness` recognizes exact canonical embedded payload hashes from the pinned baseline and 0.1.10, not upstream project-specific root docs. It reuses `install_plan` and the transaction engine; preserved legacy inputs are guarded as no-op preimages. Public CLI/package smoke covers saved preview/apply, local edits, outer bytes, legacy retention, drift/corrupt-baseline rejection and unknown refusal. Existing AgentFoundry state blocks re-migration. Latest package identity and representative recovery proof are in milestone evidence.

Latest AF-015 evidence also covers 0.1.10 wisdom/Claude migration and a real process crash after AGENTS replacement followed by public rollback, exact legacy restoration and successful normal-package migration. Exhaustive migration crash/native coverage remains open; latest package identity is in milestone evidence.

AF-002/003 continuation: distribution, entry and installation-state DTOs now live in `models.rs`; pure entry and installation-state validation moved out of composition. `InstallationState::parse` bounds and validates metadata before baseline filesystem reads. Build-time manifests reject unknown root/entry keys, matching schema policy. Public migration now refuses either orphan Harness delimiter in CLAUDE before mutation instead of silently omitting the adapter. Current package identity and gates are recorded in milestone evidence; broader application ports, sessions, release handoff and native gates remain open.

Distribution continuation: install and both migration consumers now use bounded `Distribution::parse`; shared semantic entry-set validation rejects duplicate IDs, case-insensitive path collisions and file/parent collisions. Direct model regression exercises invalid paths, unknown schema/fields and control-byte limits. AF-002 remains partial for the broader proposal contracts.

Snapshot reader continuation: manifest capture uses the shared bounded semantic parser before generation binding. Snapshot integration regression now uses complete installation metadata and rejects a matching-generation manifest with a corrupt binary digest. Broader prepared-plan/session/schema integration remains open.

Present snapshot configuration now passes `Config::parse` before fingerprinting; the integration check covers malformed metadata and valid whitespace-only config drift separately. This does not complete persisted prepared-plan or resolution-session contracts.

Saved-plan resource-limit classification now consistently reports `INPUT_LIMIT_EXCEEDED` (exit 5), including files rejected from metadata before reading. Existing packaged public scenario asserts the code and unchanged consumer bytes/state.

Stored-plan recovery continuation: hydration prevalidates all payload/image digests, portable targets and file modes before backup reads. Large-transaction regression now covers invalid metadata as well as corrupt backup content and unchanged consumer bytes. Complete pure prepared-plan/journal models and cross-platform proof remain open.

Stored-plan metadata prevalidation includes case-insensitive duplicates and file/parent collisions; the recovery regression covers each refusal. These checks precede backup image hydration and do not replace the transaction internal-target allowlist.

AF-006 continuation: apply publishes `Prepared` durably, then `Applying`, before owned-file mutation. Recovery accepts interrupted preparation; the public process-crash sweep adds `prepared-durable` and verifies rollback, retained backups and outside sentinel bytes.

AF-006 commit checkpoint continuation: durable `CoreCommitted` now precedes `Complete`; manifest-bearing recovery continues to trust exact manifest bytes rather than phase alone. Public process-crash coverage adds the committed-checkpoint boundary with preserved post-commit local edits.

Journal phases now use a strict typed enum with unchanged wire names. Generic committed recovery has a regression preserving subsequent consumer edits and idempotence; manifest-bearing recovery separately rejects phase-only commit evidence. Full activation/rollback lifecycle remains incomplete.

Journal model continuation: the DTO and strict phase enum moved to `models.rs`; application imports the shared crate-internal contracts. Public corruption coverage explicitly rejects unknown phases while retaining journal, backup and consumer bytes. This ownership cutover does not complete all journal fields required by the proposal.

Journal parsing continuation: bounded `Journal::parse` owns schema/ID/root/phase/plan-hash checks; it also rejects an embedded plan bound to another root before backup access. Pure parser regression covers identity/unknown-phase/resource refusals. Recovery keeps filesystem/content authorization and manifest witness checks.

Journal parser now invokes shared pure stored-plan validation before backup access. Hydration reuses the same validator; no duplicated metadata policy. Rehashed invalid nested schema/payload metadata is covered by the parser regression. Complete proposal plan fields, activation and native release gates remain open.

AF-006 rollback continuation: recovery publishes durable typed `RollingBack` only after validating the full rollback input set and before restoring targets. Public process-crash coverage interrupts at this durable boundary and then resumes recovery; dry-run retains the prior phase. Journal schema includes the phase. Full activation/rollback terminal-state contracts remain open.

Rollback completion publishes `Recovered` before journal removal. Public recovery scenario interrupts after both RollingBack and Recovered publication, then checks repeat recovery and exact original absence. These checkpoints do not add activation repair or close native durability gates.

Stored-plan validation now rejects incompatible merge/replace choices and changed operations after the manifest commit point before image reads. Existing large-transaction regression covers both refusals; direct in-memory application plans retain their authorization checks.

Recovery no longer rewrites preserve-only operations. The every-prefix integration check includes a preserved consumer file and asserts stable Unix inode across rollback/commit; deletion coverage also refuses consumer recreation before any rollback write.

Journal metadata validation now applies the same pure internal-target allowlist as direct application plans before image hydration. Stored baseline object references must equal their target digest, including any preimage. Rehashed journal regressions reject runtime targets, case-alias control paths and mismatched immutable objects without filesystem access. Broader AF-002/006 and release gates remain open.

T-035 now has a real non-root POSIX permission-denied public recovery scenario, not only an injected failure: retained journal/backup/target bytes, explicit partial recovery result and successful retry after restoring directory permissions. Disk-full and Windows sharing-violation scenarios remain open.

Install planning now bounds final managed outputs to the same 8 MiB limit used by update/snapshot readers. Previously, appending a managed block to an exactly 8 MiB AGENTS container produced a successful installation that subsequent offline update could not read. Packaged acceptance now exercises the boundary and requires refusal before control-state creation, preserving the original container.

The final-output check is shared by install operations, including user-owned ignore containers, control JSON and executable bytes, using the existing reader limits. Packaged acceptance also freezes an exactly 8 MiB root gitignore and requires refusal without writing ignore rules or installation state.

Offline update now treats an installed managed block missing from its still-present consumer container as a structural conflict, rather than silently reporting no-op. It uses the existing block parser to distinguish absent from malformed markers and preserves the entire container; explicit shim refresh remains the repair choice. Public acceptance covers both preview and apply refusal without consumer changes.

Three-way merge now classifies B present/L absent/U=B as missing_managed before the upstream-unchanged shortcut. The table regression failed before the fix; migration acceptance removes a mapped legacy source and requires conflict/no mutation, then restores it before the normal migration proof.

Snapshot reader now automatically captures config/manifest even when the caller requests only managed files. The existing snapshot integration check verifies config drift and malformed control data through that smaller request as well as explicit full-path capture. Complete prepared-plan/session integration and broader AF-002/003 remain open.

Snapshot schema/model now reject omitted config/manifest entries even when their digest is null. Reader regressions cover absent-control creation as drift; session fixtures explicitly freeze absent controls rather than silently leaving them out. This completes that control-input invariant, not the broader prepared-plan/session or release gates.

Installed-state schema/model now reject empty version/source identities before baseline reads. Public status and reinstall acceptance require refusal with unchanged consumer snapshot; these fields remain development build identities, not claims of stable publisher provenance.

Public migration apply now recovers pending transactions under the operation lock before legacy detection and plan construction; migration inventory/dry-run do not mutate recovery state. Packaged migration proof includes an independently interrupted fixture, no-write dry-run refusal, and direct apply retry completing migration while retaining legacy bytes.

Interface outcome composition now preserves automatic recovery mutation when subsequent install/update/migration planning fails or returns no-op. Packaged acceptance rebuilds a pending journal for each command, forces a later validation/read error and checks rolled_back plus restored bytes and cleared journal.

Saved-plan/root continuation: plans now require explicit nullable root identity and include it in semantic IDs. Existing Unix roots bind canonical path/device/inode; absent targets bind after lock creation into stored-plan/journal evidence. Recovery checks identity before backup access. Public packaged acceptance refuses byte-identical root replacement before control creation; recovery regression retains evidence on replacement and succeeds after restoring the original directory. Schemas/constructors/serialized recovery fixtures migrated without compatibility fallback. Non-Unix remains path-only; native file IDs, complete prepared sessions and broader release gates remain open.

AF-002/009 frozen-plan integration: `Plan.snapshot` is required nullable on the wire and participates in semantic ID/equality. Install/migration freeze every operation input and both controls; offline update retains its captured full snapshot. Apply validates exact path membership and binds each snapshot fingerprint/mode to the operation preimage before revalidating under lock. Absent-root plans bind and freeze after directory creation. Stored recovery plans intentionally retain image-based validation, not stale pre-apply snapshots. Public packaged scenarios cover omitted snapshot, extra/missing/duplicate frozen paths and valid config-byte drift. Conflict sessions/public continue/abort and release candidate handoff remain unimplemented; no session acceptance is claimed.

Owner confirmed GitHub release repository `anquocviet/agent-foundry`. `status --check-updates` now explicitly observes its stable release API and `agentfoundry-release.json`, validating HTTPS redirect origins, tag binding and exact 11-asset inventory with bounded reads/timeouts. Ordinary status remains offline. The actual packaged command returned RELEASE_UNAVAILABLE/exit 7 for HTTP 404 with unchanged consumer snapshot; positive published-release/candidate execution remains unverified. No arbitrary endpoint or release-root environment override is exposed. Release binaries strip symbols to keep inline saved-plan roundtrips within existing limits; external large-plan storage remains open.

Transaction commit guard rechecks processed after-images/preserve-only inputs before manifest publication (or generic committed phase), including recreated deletions and newly created output modes. External changes enter recovery with unknown bytes retained. Commit guard regression and full packaged CLI/crash gates passed; persisted conflict sessions remain incomplete.

AF-009 continuation: real distinct-build managed-file and managed-block stage/public status/abort/preview/continue passed, including resolved bytes and exact target executable digest. Session retains frozen inputs, pinned release metadata for stable candidates and existing StoredPlan/blob images. Installed baseline blobs are preserve-only plan inputs; implicit path migrations are rejected. PAR-16 and T-021/023..028 remain partial: full tamper/crash/native matrix and positive published candidate handoff remain open. `examples/check-session.rs` retains the runnable check; final local premerge and packaged lifecycle passed. Do not mark AF-009 or v1 complete.
