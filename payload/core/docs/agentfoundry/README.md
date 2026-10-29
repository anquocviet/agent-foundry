# Repository documentation map

Start with the smallest authoritative surface. This directory is AgentFoundry protocol, not documentation of the consumer application's architecture.

## Installed protocol

- [WORKFLOW.md](WORKFLOW.md): authority, missing capabilities, trust and evidence.
- [Invariant pattern](patterns/encoding-invariants.md): encode accepted rules with native checks.
- [Product documents](product/README.md): optional locations for consumer-owned product intent.
- [Decisions](decisions/README.md): optional durable decision records.
- [Plans](plans/README.md): optional requested plans, without a required task lifecycle.
- [Decision template](templates/decision.md), [execution plan](templates/exec-plan.md), [runbook](templates/application-runbook.md), [improvement experiment](templates/harness-improvement.md).

## Consumer authority

Root README, applicable AGENTS instructions, existing product/architecture documents, code, tests and CI remain their original owners. Their existence and operating commands are **Unknown** until observed in this repository. Use explicit read-only `agentfoundry inspect` to inventory candidates; it does not execute application commands or establish business policy.

No `crates/harness`, upstream architecture document, SQLite state, source-repository workflow or release tooling is assumed to exist in a consumer. Legacy files are not deleted by installation. Skills require explicit invocation; permission to inspect is not permission to modify or operate the application.
