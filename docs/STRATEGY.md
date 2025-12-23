# isq Strategy Kernel

## Winning Aspiration

isq becomes the infrastructure layer for issue tracking—the way developers and AI agents access issues, regardless of which tracker stores them.

## Where to Play

**Users**: Developers who live in the terminal, and their AI agents. The whole team accesses issues through agents; isq is invisible infrastructure.

**Forges**: GitHub (open source default), Linear (quality benchmark), then Forgejo/GitLab (self-host), eventually Jira (enterprise).

**Stages**: The entire development loop. isq provides the data and actions; the interface is CLI for humans, structured output for agents.

## How to Win

**System-level integration beats API wrappers.**

Linear's MCP is a REST API for chat. isq is a daemon with local state, git integration, and offline capability. This enables things API wrappers cannot:

- Know which issue you're working on (inferred from worktree/branch)
- Work offline, sync later
- React to git events
- Sub-millisecond reads from local cache

**One tool, any backend.** Same commands for GitHub, Linear, Forgejo. Your workflow survives platform migrations.

**Insanely great for humans = great for agents.** We don't design separately. Speed, reliability, and composability serve both.

## Capabilities

**Must be world-class**: Speed. Reliability. Git integration. Forge abstraction. Structured output.

**Explicitly not building**: Native dashboards, rich text editing, mobile apps. Agents generate artifacts on demand.

## The Bet

If AI agents become the primary interface for dev tools → isq wins big (system integration beats API wrappers).

If they don't → isq still wins (great CLI serves humans directly).

We build something great for humans that's even better with AI agents. That's the asymmetry.
