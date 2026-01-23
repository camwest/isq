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

Problem: Only works with GitHub/Linear. Doesn't fulfill "any backend" promise.

*Update: JIRA Cloud support added (OAuth + API token auth). GitHub, Linear, and JIRA now supported.*

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

**Tangled Backend** — If atproto-native forges gain traction, support Tangled as a first-class forge.

-----

## Monitoring

*Things we're watching rather than building. Integrate when mature.*

**Tangled & AT Protocol** — Tangled (tangled.sh) is building atproto-native git collaboration with decentralized issues. If they succeed, isq becomes a CLI/daemon for atproto-native forges rather than building owned storage ourselves. Watch their lexicon choices, federation model, and adoption.

**Radicle** — P2P code collaboration (not AT Protocol). Custom gossip protocol, 7+ years in development, token-funded. Different bet on adoption path. Monitor but lower priority than Tangled.

-----

*Later is direction, not delivery. We act, learn, adjust.*