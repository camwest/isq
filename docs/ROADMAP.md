# Roadmap

**Mountain**: isq becomes the infrastructure layer for issue tracking—the way developers and AI agents access issues, regardless of which tracker stores them.

---

## Now

**Production-Ready + Git Integration**

Problem: isq isn't reliable enough for daily use, doesn't understand dev context.

- Bug fixes, error handling, daemon reliability
- Git context: detect worktree/branch → infer current issue
- `isq` with no args shows context
- `isq start <id>` / `isq done <id>` lifecycle
- GitHub releases, install script, docs

Exit criteria: You use isq daily without hitting bugs.

---

## Next

**Multi-Repo & Personal State**

Problem: I work across repos, no unified view. I lose track of what I'm working on.

**Universal Forge**

Problem: Only works with GitHub/Linear. Doesn't fulfill "any backend" promise.

---

## Later

**Triage at Scale** — Open source maintainers drowning in issues.

**Team Visibility** — I can see my work, but not my team's.

**PR Integration** — Issues and PRs are disconnected.

**Workflow Automation** — I do the same issue workflows manually.

---

*Later is direction, not delivery. We act, learn, adjust.*
