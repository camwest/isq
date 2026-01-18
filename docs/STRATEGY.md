# isq Strategy Kernel

## Winning Aspiration

isq becomes the infrastructure layer for issue tracking — a data layer you own, so developers and AI agents can work without rent-seekers in the loop.

## Where to Play

**Users**: Developers who live in the terminal, and their AI agents. The whole team accesses issues through agents; isq is invisible infrastructure.

**Forges**: GitHub (open source default), Linear (quality benchmark), JIRA Cloud (enterprise), then Forgejo/GitLab (self-host). Today these are sync sources. Tomorrow they’re optional sync targets.

**Stages**: The entire development loop. isq provides the data and actions; the interface is CLI for humans, structured output for agents.

## How to Win

**System-level integration beats API wrappers.**

Linear’s MCP is a REST API for chat. isq is a daemon with local state, git integration, and offline capability. This enables things API wrappers cannot:

- Know which issue you’re working on (inferred from worktree/branch)
- Work offline, sync later
- React to git events
- Sub-millisecond reads from local cache

**One tool, any backend.** Same commands for GitHub, Linear, JIRA, Forgejo. Your workflow survives platform migrations.

**Ownership removes the toll booth.** When AI agents do the work, whoever controls the data layer extracts rent. GitHub charges for Copilot. Linear charges per seat. Every API call flows through their pricing. Owned data breaks that — agents work on local data, productivity gains accrue to you.

**Insanely great for humans = great for agents.** We don’t design separately. Speed, reliability, and composability serve both.

## Capabilities

**Must be world-class**: Speed. Reliability. Git integration. Forge abstraction. Structured output.

**Building toward**: Portable issue format. User-owned storage. Rent-free agent operations.

**Explicitly not building**: Native dashboards, rich text editing, mobile apps. Agents generate artifacts on demand.

## The Bet

If AI agents become the primary interface for dev tools → isq wins big (owned data beats API wrappers, rent-seekers get disintermediated).

If they don’t → isq still wins (great CLI serves humans directly).

We build something great for humans that’s even better with AI agents, on a data model that’s even better when you own it. That’s the asymmetry.