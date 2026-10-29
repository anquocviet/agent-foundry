<!-- AGENTFOUNDRY:BEGIN -->
## AgentFoundry

Start with the requested outcome and use the repository as the system of record.
Find the relevant code, interfaces, commands, tests, and evidence through the
repository map in `docs/agentfoundry/README.md`. `docs/agentfoundry/WORKFLOW.md` describes repository
understanding, missing capabilities, authority, and trust.

- Respect the requested scope. Inspection does not authorize edits or operation.
- Prefer source references over duplicate summaries. Preserve product intent
  and rationale that code and tests do not express.
- Identify missing capabilities needed to act, observe, or verify. Report what
  is unavailable or unproven instead of inventing commands or claiming success.
- Before changing externally observable policy, establish repository authority.
  Stop at material unresolved choices; configurable defaults are not authority.
- Preserve user-owned state and use the repository's native safety and validation
  checks. Explain observed results and their limits.
- Only invoke a AgentFoundry skill when the user explicitly requests it.

Consumer repositories own their task workflow and planning. AgentFoundry provides
context and optional tools, without a task database or orchestration lifecycle.
<!-- AGENTFOUNDRY:END -->
