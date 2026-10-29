# Architecture decisions

Proposal defaults adopted, not a scope reduction. ADR-001 through ADR-012:


| ADR | Mặc định đề xuất để agent bắt đầu | Lý do và ràng buộc |
| --- | --- | --- |
| ADR-001 | Rust cho CLI/core; một workspace, một executable public | Kế thừa lợi thế binary đa nền tảng, types và testability |
| ADR-002 | Domain → application ports; adapter filesystem/process/network bên ngoài | Policy dễ test bằng fake ports; main là composition root |
| ADR-003 | Payload khai báo bằng manifest, embedded lúc build | Một nguồn danh sách file; không liệt kê thủ công hai nơi |
| ADR-004 | Bash/PowerShell chỉ bootstrap verified binary và forward args | Mọi mutation của repository đi qua transaction engine |
| ADR-005 | JSON schema versioned, content-addressed upstream blobs | Kiểm tra provenance, snapshot, migration và lỗi có cấu trúc |
| ADR-006 | Shared entrypoint dùng marker block; docs riêng namespace | Hạn chế xung đột với tài liệu sở hữu bởi ứng dụng |
| ADR-007 | Git merge-file làm merger text v1 | Không tự viết merge algorithm mới; phân biệt conflict với lỗi |
| ADR-008 | Evidence tooling v1 giữ Python stdlib, port có chọn lọc | Giữ capability, giảm rủi ro rewrite parser; maintenance CLI không cần Python |
| ADR-009 | Release GitHub immutable; checksum + release metadata + source identity | Chưa gọi checksum là independent publisher authentication |
| ADR-010 | Lock hợp tác + snapshot validation + journal + atomic replace từng file | Multi-file transaction recoverable, không tuyên bố filesystem ACID tuyệt đối |
| ADR-011 | Migrate Harness bằng mapping sang namespace mới, giữ nguồn cũ | Tránh hai updater sở hữu cùng file |
| ADR-012 | Version phát hành sản phẩm đủ scope là 1.0.0; build nội bộ dùng prerelease | Không gắn nhãn v1 cho sản phẩm còn thiếu hàng parity |

Không hardcode version crate theo trí nhớ. Trong AF-001, chọn/pin Rust toolchain, MSRV, Python minimum và dependency versions bằng build trên platform matrix; commit `rust-toolchain.toml`, `Cargo.lock`, `rust-version` và bảng prerequisites. Giữ số dependency thấp; đánh giá license, bảo trì và platform behavior của crate được chọn.


## Toolchain decision

Rust/MSRV 1.90.0, edition 2024; Python minimum 3.10 (native matrix pending). serde/serde_json for control DTOs, sha2 for SHA-256, fs2 for OS advisory locks. Exact resolved versions in Cargo.lock. No runtime HTTP dependency until release origin is configured. Local Linux proof only; platform verification pending.

## Durability ceiling

Cooperative locking cannot exclude editors. Atomic per-file rename is not whole-filesystem isolation. Parent-chain checks reduce observable races, not a sandbox against a same-account attacker. Windows replacement adapter requires native verification before support claims.

## Semantic plan identity

`agentfoundry.plan-canonical/v1` hashes a compact JSON tuple of canonicalization version, plan schema, canonical root, root identity, full frozen snapshot, payload digest, fixed-order choices and ordered operation rows `(target, before SHA-256 or null, after SHA-256 or null, mode)`. Operation order is semantic because manifest publication is the commit witness. Timestamps and presentation formatting are excluded; content hashes use exact bytes. Public install/update report the recomputed ID; saved-plan authorization still validates paths, preimages and allowed outputs rather than trusting an ID as a write capability.

## Distribution trust boundary

Build-time distribution parsing rejects unknown root and entry fields, matching the versioned schema and runtime Serde policy. Misspelled selectors must fail rather than silently change install selection. Canonical payload identity still excludes build-only source paths; strict validation does not change identity for valid manifests.

Distribution and entry DTOs live with the pure models, not the composition module. Install and migration import the same `models::Distribution`/`models::Entry`; no root-level compatibility re-export remains. Wire fields and payload bytes are unchanged.

The installation-state DTO also lives in `models::InstallationState`; composition retains I/O and baseline checks, while install/migration serialize the shared DTO. The former composition-local `State` type is removed.

Pure entry validation now resides beside the DTOs in `models.rs`; composition calls it before baseline filesystem reads. This separates ownership/path/selector checks from I/O without duplicating validation policy.

`InstallationState::validate` owns schema/generation/digest/entry-set validation without filesystem reads. `load_state` validates the complete metadata first, then verifies baseline blobs through the filesystem adapter. Invalid path sets therefore fail before any baseline object access.

`InstallationState::parse` bounds control bytes before deserialization and applies semantic validation; the filesystem loader uses this entrypoint. A direct model regression covers invalid generation/schema/digest and oversized input.

`Distribution::parse` applies bounded strict deserialization and validates the complete entry set before install or migration reads consumer state. Distribution and installation metadata share entry-set checks for logical IDs, case-folded targets and file/parent collisions; composition no longer repeats per-entry checks.

Snapshot capture parses manifest bytes through `InstallationState::parse`, not an untyped schema/generation projection. Invalid metadata cannot become a trusted frozen input merely because its generation matches. Generation drift remains `WORKSPACE_DRIFT`; malformed metadata retains the parser's validation error.

Snapshot capture also validates present configuration through `Config::parse` before fingerprinting. The snapshot regression distinguishes invalid configuration from a valid byte-only config change: invalid input fails validation; trailing-whitespace drift remains detectable by exact-byte fingerprints.

Saved-plan metadata size rejection returns `INPUT_LIMIT_EXCEEDED`, matching the bounded-read overflow branch and proposal resource contract; unsafe file kinds remain `INVALID_STATE`. Both refusals precede consumer mutation.

Stored-plan hydration validates payload/hash syntax, portable targets and supported modes for the entire operation list before reading backup images. Corrupt metadata fails `RECOVERY_BLOCKED` before image allocation; content hashes are still verified after bounded reads, and transaction authorization remains in the application validator.

Hydration also rejects case-folded duplicate targets and file/parent collisions before reading any image. Recovery never allocates a repeated image set for ambiguous operations, even if all supplied digests are syntactically valid.

Durable journal phases are a strict Serde enum (`Prepared`, `Applying`, `CoreCommitted`, `Complete`), replacing free-form strings and a separate acceptance list. Unknown phases fail deserialization as `RECOVERY_BLOCKED`; wire names remain stable. Lifecycle activation and rollback checkpoints remain separate unfinished contracts.

Journal DTO and phase enum now live in `models.rs` with crate-internal visibility. The application owns filesystem checks, backup verification and recovery execution; no composition-local duplicate DTO or compatibility alias remains.

`Journal::parse` bounds control bytes, strictly decodes phases and binds the journal/embedded plan to the expected root and canonical stored-plan hash without filesystem reads. Recovery then checks exact backup metadata and hydrates images. No duplicate composition validator remains.

Stored-plan semantic validation is now an I/O-free method shared by journal parsing and hydration. Journal parsing validates the complete nested metadata before backup access; recomputing the plan hash does not authorize an unknown nested schema or malformed payload digest.

Pure stored-plan validation rejects mutually exclusive merge/replace choices and mutations ordered after a changed manifest commit before any image hydration. Application validation retains the same checks for in-memory plans; stored metadata cannot defer these invariants until after backup reads.

Transaction internal-target policy lives in the pure domain layer and is reused by application validation and stored-plan metadata parsing. Recovery rejects undeclared control targets and immutable-object digest/reference mismatches before reading backup images; direct apply additionally hashes actual object bytes.

Managed-file resource bounds apply to final planned bytes, including consumer prefix/suffix around managed blocks, not only input reads. Install refuses an over-limit result before transaction staging so it cannot publish a state the maintenance readers reject.

The shared install operation builder enforces output limits for payloads, ignore containers, control documents and executable bytes: 8 MiB text, 16 MiB control JSON, 128 MiB executable. Appending ignore rules cannot create an unreadable maintenance input either; boundary failures happen before consumer mutation.

Same-distribution update validates owned block presence, not only marker syntax. A container with no delimiters is a missing-owned-block structural conflict; it is neither a valid preserved block nor authorization to append one during update. Local edits within a valid block and outside it remain preserved.

Missing-managed structural conflicts take precedence over the upstream-unchanged fast path: B present, L absent, U present is a conflict even when U equals B. Preserve absence only when U is also absent. This prevents migration from silently dropping tracking for a consumer-deleted source that still exists upstream.

Snapshot collection always freezes config and installation-manifest presence/bytes in addition to caller-selected managed paths. Callers cannot accidentally omit generation/config guards by passing only payload paths; combined path count remains bounded at 10,000. Existing explicit control paths are not duplicated.

Snapshot wire validation requires explicit records for both control paths, even when absent. A null control digest cannot mean an omitted input. The JSON schema requires both records and pure model validation checks presence/fingerprint consistency; resolution fixtures carry the same frozen control set.

Public mutation reporting retains a completed automatic-recovery outcome across later planning errors or no-op results. A command that rolled back files cannot report mutation none merely because the subsequent saved-plan read or migration validation failed. More specific later mutation/error outcomes take precedence.

Saved plans and stored transactions reuse snapshot root identity (Unix device/inode plus canonical path). Nullable saved identity explicitly represents an absent root; after lock creation apply binds the actual created directory into stored evidence. Recovery requires a nonempty identity and checks it before backup hydration. This is a development wire cutover without legacy fallback; Windows native file IDs remain a release blocker.

Release authority confirmed by the owner: GitHub repository `anquocviet/agent-foundry`. Release clients must pin that repository, not the upstream Harness origin or consumer configuration. A read of `https://api.github.com/repos/anquocviet/agent-foundry/releases/latest` returned HTTP 404; this does not distinguish an absent public release from a private/unavailable repository. No credentials, remote settings, push or publication were performed. Candidate acquisition/activation remains blocked until implemented and verified; the origin decision alone is not release provenance.

Pinned release metadata observation uses synchronous `ureq` with Rustls (no curl/Python runtime dependency), HTTPS only, bounded response reads and manual allowlisted redirects. Metadata asset name is `agentfoundry-release.json`; GitHub inventory must match five binaries, five sidecars and that metadata. Release profile strips non-runtime symbols; safety input limits remain unchanged. This read-only observation neither executes candidates nor completes AF-010/011.
