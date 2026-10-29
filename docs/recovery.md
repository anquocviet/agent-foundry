# Development recovery guide

Do not delete `.agentfoundry/runtime/transaction.json` or `.agentfoundry/backups/` to bypass an error. Backups contain private consumer bytes; keep them local. There is no automatic retention cleanup.

Saved install plans also contain private preimages. Unix plan outputs are created exclusively with mode `0600`; keep them outside the consumer and do not commit or upload them. Preview rejects a serialized plan above the 16 MiB reader limit before creating its output. Windows ACL confidentiality remains subject to the host directory policy and native verification gate.

```sh
agentfoundry recover --directory /disposable/consumer --dry-run --json
agentfoundry recover --directory /disposable/consumer --json
```

Recovery dry-run validates current byte/mode preconditions and committed baseline integrity before reporting a feasible rollback/finish. Drift returns `RECOVERY_BLOCKED` without writes. A failure after recovery has attempted a restore or journal removal reports `partial_recovery_required`; preserve all evidence and retry only after resolving the reported condition.

Install enters the recovery gate before decoding the installed manifest. Status/doctor do not recover. Recovery validates journal root/hash, backup equality and declared paths. Before core commit, only exact transaction images with expected ordinary permission bits are restored; unknown external byte or permission edits return `RECOVERY_BLOCKED` without overwrite. Manifest after-image is the commit witness even when journal phase is stale. Recovery can be repeated. Preserve backup/journal and manually reconcile unknown data before retrying.

Saved plans require an explicit nullable root identity: null authorizes an absent target only. Existing Unix roots bind canonical path plus device/inode; byte-identical replacement directories fail with WORKSPACE_DRIFT before lock creation. Apply binds a newly created root before persisting the journal; recovery rejects a different root with RECOVERY_BLOCKED before reading backups. Old development plans/journals without this field are rejected, not silently upgraded. Preserve interrupted roots and their evidence; copying a journal to another directory does not authorize recovery there. Non-Unix identity remains canonical-path-only pending native file-ID support.

Saved install, migration and offline update plans now require an explicit nullable `snapshot`. Existing roots freeze the exact operation input set plus config/manifest, including generation, presence, hashes and ordinary modes; null is reserved for an absent fresh-install root. Apply rejects omitted, extra or duplicate frozen paths and snapshot/preimage inconsistencies, then rechecks the snapshot under the operation lock. Recovery intentionally validates transaction pre/post-images rather than the pre-apply snapshot, since interrupted writes have already changed it.

Immediately before publishing a changed installation manifest, apply rechecks processed outputs and preserve-only inputs against after-images/modes, including intended absence after deletion. Generic transactions perform the same check before the committed phase. Observed editor changes enter recovery without overwriting unknown data; cooperative-writer races are reduced, not eliminated.

Before any owned-file mutation, apply publishes durable `Prepared` and then `Applying` journal phases. An interruption between them is recoverable using the same verified backup and preimage rules; a prepared journal is not completion evidence.

After all core operations, apply publishes `CoreCommitted` before `Complete`. Installed-state transactions still use exact manifest after-bytes as the commit witness, not the phase alone; post-commit consumer edits remain valid. Generic transactions without a manifest use the durable committed phase.

Recovery validates the full rollback input set before publishing durable `RollingBack`; only then does it restore target preimages. An interrupted rollback revalidates bytes/modes and resumes idempotently. Preview never advances the journal phase.

After restoring all rollback preimages, recovery publishes `Recovered` before removing the active journal. A crash at this boundary leaves verified backup evidence; retry revalidates the restored inputs and finishes journal cleanup.

Preserve-only operations are revalidated but never replaced during rollback. Unchanged consumer files retain their inode and filesystem metadata rather than being rewritten merely because their before/after bytes match.

## Development limitations — release blockers

- Saved install and offline update plans carry inline hexadecimal byte images and are bounded at 16 MiB. Transaction journals/backups use separately verified content-addressed images; control JSON stays bounded. Large saved-plan external staging remains required.
- Locking is cooperative; editors do not participate. Path-chain checks and immediate preimage checks do not eliminate every symlink-swap/check-rename race. No attacker sandbox claim.
- Original bytes and Unix ordinary permission bits are preserved; ACLs, ownership, special permission bits and directory preimages are not fully modeled. Created empty directories may remain after rollback.
- Atomic replacement plus Unix file/directory sync is implemented; no whole-filesystem isolation or power-loss proof. No NFS/SMB/WSL Windows-mount guarantees.
- Windows reparse rejection is present but Windows replacement/sharing/activation and macOS have no native proof. Do not claim platform support from conditional compilation.
- Executable currently participates before manifest commit; separate activation-pending repair state is not yet implemented. No self-update support.
- Journal root/hash and backup content are verified and mutation targets have an internal allowlist, but this is not authentication against malicious same-account edits. Complete generation/config/mode snapshots, exhaustive crash coverage and strict architectural port boundaries remain required.
- Dry-run does not create target files. Explicit plan-out writes outside the target with exclusive creation; plain preview does not write temp files. Merge adapter uses OS temp only when called; it is not exposed by read-only inspect.
- No release origin, downloader or publish permission is configured. Checksum sidecars are consistency checks, not publisher signatures.

A failed or incomplete gate means this remains a development build. See the full backlog before expanding use.

## Subsequent hardening

Internal mutation targets now have an exact state/config/object allowlist; object hashes must match bytes. Frozen ordinary permission bits, root symlink/kind, nondirectory parents and nested Git/submodule boundaries are rejected. Descriptor-relative race protection, directory preimages and large saved-plan external storage remain incomplete. Config is strictly validated and optional selections persist. Explicit-feature process crashes now exercise journal, file, manifest and interrupted rollback boundaries; default packages reject that feature. These changes do not close the release blockers above.

Migration uses the same journal/backup engine and leaves legacy state, binaries, docs and skills untouched. If preview reports a text/structural conflict, no migration session is staged: resolve the legacy condition and replan. Do not run the old updater concurrently. A saved migration plan freezes legacy manifest/base/source bytes as well as destinations; drift requires a fresh preview. After successful migration, rerunning reports an existing AgentFoundry installation instead of overwriting it. Legacy cleanup is never automatic; select the new namespaced skills explicitly to avoid duplicate discoverability.

Explicit migration apply enters the recovery gate before legacy detection/plan construction, matching install/update. Inventory and dry-run remain read-only and do not recover a pending journal. After a recoverable interruption, rerun the explicit migration apply; unknown drift or corrupt recovery evidence still blocks it.

Cross-release text sessions retain a hashed StoredPlan, frozen snapshot, verified binary images and exact BASE/LOCAL/UPSTREAM under local runtime/backup storage. `status` shows RESOLVED paths. `update --continue --dry-run` validates without applying; `update --continue` applies under the transaction lock; `update --abort` removes declared session artifacts without changing installed files. Backup images remain for recovery. Unresolved session-specific markers block continue. Managed blocks merge their owned bytes only; resolved blocks must retain valid ownership delimiters and are patched into the frozen container, preserving prefix/suffix bytes. Windows online activation remains unavailable.

After a committed session interruption, `update --continue` verifies the active repo-local executable digest and Unix execute permission before clearing the session, including in preview. A missing, changed or non-executable binary returns `RECOVERY_REQUIRED` and retains evidence. With no pending transaction, `recover --dry-run` previews restoration of a missing executable from the pinned session image; `recover` restores it through the transaction engine, runs `version --json`, and verifies version/source/payload identity. It does not fetch latest, change core files, clear the resolution session or overwrite unknown executable bytes. Use `update --continue` afterward to finish the retained session. Existing non-executable bytes require explicit permission restoration; native Windows activation remains incomplete. Session root identity is checked before hydrating retained images.

Repair transactions preserve the installed manifest rather than changing its generation. Before their durable commit checkpoint, interrupted repair rolls back the created executable; rerun `recover` to restore it again from the session. After `CoreCommitted`, matching unchanged manifest plus the durable phase proves transaction completion: recovery retains the executable. A missing retained candidate blocks without creating an executable or clearing the session. `AF_CHECK_REPAIR_CRASH` in the runnable session check exercises these repair-specific checkpoints.

After a committed conflict session, `update --abort` refuses while its pinned executable is missing, altered or non-executable; it retains candidate evidence. Clean online updates use an independent activation record: recover an interrupted core journal first, then run `recover` again to preview or complete activation. Unknown executable bytes are retained, never overwritten. Windows activation remains unavailable.

If `recover` replaces the executable but its subsequent identity query, JSON parsing, identity validation or record cleanup fails, the error reports `mutation: applied`; the activation record remains for inspection/retry. If no replacement occurred, an identity-query failure reports `mutation: none`.
