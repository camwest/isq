# Strategy Kernel: isq

*Playing to Win framework*

---

## 1. Winning Aspiration

**isq becomes the universal developer interface for issue tracking—the layer between developers and any issue system.**

Success looks like:
- Developers instinctively run `isq` instead of opening a browser
- AI coding agents (Claude Code, Cursor, Aider) use isq as their issue interface
- "isq" becomes a verb: "just isq it" (like "google it" or "grep for it")
- Open source projects adopt isq as the recommended way to interact with their issues
- Platform migrations (GitHub → Forgejo, Linear → something else) become trivial because developers use isq, not the native UI

**The 10-year vision**: Issue trackers become commodity backends. isq is how developers experience issues, regardless of where they're stored.

---

## 2. Where to Play

### Primary Market: Individual developers who live in the terminal

**Who:**
- Professional developers who use terminal as primary interface
- Open source maintainers managing high-volume issue triage
- Developers using AI coding agents (Claude Code, Cursor, Aider)
- Teams that care about speed and keyboard-driven workflows

**Who we're NOT targeting (initially):**
- Product managers (they need rich UI for planning)
- Executives (they need dashboards and reports)
- Non-technical users (they need GUI)
- Teams that want isq to replace their issue tracker (we complement, not replace)

### Forge Coverage

| Forge | Priority | Rationale |
|-------|----------|-----------|
| GitHub | P0 | Default for open source, largest market |
| Linear | P0 | Best-in-class UX, proves we can match quality |
| Forgejo/Gitea | P1 | Self-host crowd, GitHub refugees |
| GitLab | P1 | Enterprise + self-host |
| Jira | P2 | Enterprise necessity, large market |

### Stages of Development Loop

Focus on stages where **developer is in the terminal**:

| Stage | isq Fit | Notes |
|-------|---------|-------|
| Planning (big fuzzy issues) | Low | Team activity, needs rich UI |
| Decomposition/Organization | Medium | Can help, but often collaborative |
| Coordination ("what's next?") | High | Personal view of work |
| **Execution (active coding)** | **Very High** | Developer + AI in terminal |
| **Capture (found a bug)** | **Very High** | Quick logging without context switch |
| Testing/Refinement | High | Sub-issues, scope changes |
| Code Review | Medium | PR integration opportunity |
| **Triage (process queue)** | **Very High** | Keyboard-driven queue processing |
| Support (escaped defects) | Medium | Quick bug logging |

**Primary focus**: Execution, Capture, Triage

---

## 3. How to Win

### Value Proposition

**For developers**: "Never leave your terminal to manage issues. Faster than any web UI. Works offline. Works with AI agents."

**For AI agents**: "The only way to understand and manipulate issue context during coding sessions."

**For open source**: "Linear-quality UX without the price tag. Works with any forge."

### Sustainable Competitive Advantages

#### 1. System-Level Integration (vs. API wrappers)

Linear's MCP and Slack agent are **application-layer** integrations:
- REST API calls
- Limited context (10 Slack messages)
- No awareness of local development state
- Can't observe filesystem, git, or processes

isq is **system-level**:
- Rust binary on the machine
- Background daemon with persistent local state
- Direct access to git (branches, commits, worktrees)
- Can observe and react to development environment
- SQLite cache for instant reads

**This enables things Linear MCP cannot do:**
```bash
# isq knows you're in a worktree for issue #423
$ pwd
/home/dev/project-worktrees/423-fix-auth

$ isq
Currently working on: #423 - Fix auth flow
Time in worktree: 2h 15m
Recent commits: 3

# isq can react to git events
$ git commit -m "Fix Safari cookie handling"
# Daemon notices, optionally auto-comments on #423

# isq knows your full local context
$ isq context --json  # for AI agents
{
  "current_issue": 423,
  "worktree": "/home/dev/project-worktrees/423-fix-auth",
  "branch": "fix/423-auth-flow",
  "recent_commits": [...],
  "related_issues": [401, 389],
  "time_spent": "2h 15m"
}
```

**Moat depth**: This requires a daemon, local storage, git integration—not just API calls. Hard to replicate from a web-first product.

#### 2. Offline-First Architecture

Most issue tools assume connectivity. isq assumes disconnection:
- Full issue cache in SQLite (instant reads)
- Offline write queue (sync when connected)
- Works on planes, in tunnels, on bad wifi
- Works when GitHub is down (frequent enough to matter)

**Moat depth**: Architectural decision that's hard to retrofit. Web-first products can't easily become offline-first.

#### 3. Forge Abstraction (Universal Interface)

One tool, any backend:
- Same commands for GitHub, Linear, Forgejo, GitLab
- Personal workflow survives platform migrations
- Reduces lock-in fear (easier to adopt new forges)

**Moat depth**: Network effect—more forges supported = more valuable. First-mover advantage in being "the universal client."

#### 4. AI Agent Native

AI coding agents need issue context but can't use web UIs:
- `--json` on all commands (structured output)
- `isq context` for rich local state
- Can create, update, link issues programmatically
- Daemon can observe AI agent activity

**Moat depth**: As AI coding becomes standard, the tool that's AI-native wins. Linear's MCP is reactive (AI asks questions). isq is integrated (AI is part of the flow).

#### 5. Speed as Feature

Sub-millisecond reads from local cache. No spinner, no loading state, no network latency for reads.

When everything is instant, you use it more. When you use it more, it becomes habit. When it's habit, you're locked in.

**Moat depth**: Rust + SQLite + daemon architecture. Can't be matched by browser-based tools.

---

## 4. Capabilities Required

### Must Be World-Class At:

| Capability | Why Critical |
|------------|--------------|
| **Cache coherence** | Local cache must reflect remote state accurately; stale data destroys trust |
| **Git integration** | Worktrees, branches, commits—must understand dev environment deeply |
| **Forge abstraction** | Clean trait boundary; adding forges should be easy |
| **CLI/TUI UX** | Must feel as polished as Linear's web UI, but in terminal |
| **Daemon reliability** | Background sync must "just work"—no babysitting |
| **AI agent interface** | Structured output, rich context, composable commands |

### Must Be Good Enough At:

| Capability | Why |
|------------|-----|
| Multi-repo management | Developers work across repos, need unified view |
| Triage workflows | Keyboard-driven queue processing |
| Notifications | Know when something needs attention |

### Explicitly Deprioritized:

| Capability | Why Not |
|------------|---------|
| Planning/roadmap features | Team activity, use Linear/GitHub UI |
| Rich text editing | Terminal constraint, use $EDITOR |
| Dashboards/reports | Management need, not developer need |
| Mobile | Terminal tool, desktop only |

---

## 5. Management Systems

### Development Principles

1. **Offline-first always**: Every feature must work offline, sync later
2. **Speed is non-negotiable**: If it's slow, it's broken
3. **Forge-agnostic by default**: No GitHub-specific features leak into core
4. **AI-native from day one**: Every command has structured output
5. **Composable over complete**: Small commands that chain well, not monolithic features

### Quality Gates

- **Read latency**: <50ms (cache hit), <2s (cache miss + network)
- **Write latency**: <500ms (online), <50ms (offline queue)
- **Daemon reliability**: <1 crash per 1000 hours
- **Cache accuracy**: <1% stale reads after sync

### Release Cadence

- **6-week cycles** (inspired by Basecamp's Shape Up)
- **Cooldown week** between cycles for bug fixes, polish, debt
- **One major theme per cycle** (not everything at once)

---

## 6. Strategic Choices Summary

| Choice | We Choose | We Reject |
|--------|-----------|-----------|
| Primary user | Individual developer | Teams, managers, PMs |
| Primary interface | Terminal (CLI → TUI) | Web, mobile, desktop GUI |
| Architecture | Offline-first, daemon | Online-first, stateless |
| Forge strategy | Universal abstraction | GitHub-only or fork per forge |
| AI strategy | System-level integration | API wrapper (MCP-only) |
| Pricing | Open source, free | SaaS, freemium |
| Scope | Developer workflow tool | Full project management |

---

## 7. What Must Be True

For this strategy to succeed:

1. **Terminal remains developer home**: If developers move to browser-based IDEs entirely, our advantage shrinks
2. **AI coding agents grow**: More AI agents = more need for system-level issue integration
3. **GitHub continues to degrade**: Platform frustration drives search for alternatives
4. **We execute on speed**: If isq isn't dramatically faster, there's no reason to switch
5. **Forge diversity persists**: If everyone consolidates on one platform, universal abstraction is less valuable

### Risks

| Risk | Mitigation |
|------|------------|
| GitHub builds great CLI | Focus on multi-forge, offline-first—things GitHub won't do |
| Linear builds system-level tool | Move faster, open source community, forge diversity |
| AI agents get native issue access | Be the integration layer they use |
| Terminal use declines | Unlikely for professional developers; hedge with TUI |

---

## 8. The One-Liner

**isq is the system-level interface between developers (and their AI agents) and any issue tracker—instant, offline-first, and universal.**

Not a web UI. Not an API wrapper. A tool that understands your development environment and makes issue management a side effect of coding.
