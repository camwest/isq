# Roadmap

**Mountain**: isq becomes infrastructure for AI-native development — a data layer you own, so agents work without rent-seekers in the loop.

-----

## Now

**Production-Ready + Git Integration**

Problem: isq isn’t reliable enough for daily use, doesn’t understand dev context.

- Bug fixes, error handling, daemon reliability
- Git context: detect worktree/branch → infer current issue
- `isq` with no args shows context
- `isq start <id>` / `isq done <id>` lifecycle
- GitHub releases, install script, docs

Exit criteria: You use isq daily without hitting bugs.

-----

## Next

**Multi-Repo & Personal State**

Problem: I work across repos, no unified view. I lose track of what I’m working on.

**Universal Forge**

Problem: Only works with GitHub/Linear. Doesn’t fulfill “any backend” promise.

*Update: JIRA Cloud support added (OAuth + API token auth). GitHub, Linear, and JIRA now supported.*

**Portable Data Model**

Problem: Current local cache is an implementation detail. If we want owned storage later, the data model matters now.

Design local storage as proto-lexicons. JSON schemas that could become `io.isq.issue`, `io.isq.comment`. Doesn’t require AT Protocol yet, but doesn’t preclude it.

-----

## Later

### Features

**Triage at Scale** — Open source maintainers drowning in issues.

**Team Visibility** — I can see my work, but not my team’s.

**PR Integration** — Issues and PRs are disconnected.

**Workflow Automation** — I do the same issue workflows manually.

### Adoption

**Developer Advocacy** — Community building, content, presence.

**Enterprise** — ~Jira backend~ Done (JIRA Cloud). Next: JIRA Data Center/Server.

**Flagship Projects** — Get adopted by visible open source projects.

### Ecosystem

**MCP Server** — Let other AI tools use isq as their issue layer.

**Plugin System** — Third-party extensions, custom forges.

**API Stability** — Guarantees that let others build on isq.

**Owned Storage** — isq-native storage on AT Protocol. Issues as portable records. GitHub/Linear become sync targets, not sources of truth. Agents work on local data without API tolls.

**Hosted PDS** — isq.dev hosts issue data for open source projects. Free tier, pennies marginal cost. Projects own their data; GitHub stays open for contributors.

**Open Ecosystem** — Published lexicons. Third-party apps on same data. Any agent framework builds on isq, no vendor lock-in.

-----

*Later is direction, not delivery. We act, learn, adjust.*