# Strategy Kernel: isq

*Playing to Win framework*

---

## 1. Winning Aspiration

**isq becomes the system-level infrastructure for issue tracking—the layer between humans, AI agents, and any issue system.**

Success looks like:
- Developers instinctively reach for isq (directly or through AI agents) instead of opening a browser
- AI coding agents (Claude Code, Cursor, Aider) use isq as their issue interface
- Entire teams interact with issues through AI agents backed by isq—not just developers
- "isq" becomes a verb: "just isq it" (like "google it" or "grep for it")
- Open source projects adopt isq as the recommended way to interact with their issues
- Platform migrations (GitHub → Forgejo, Linear → something else) become trivial because isq abstracts the backend

**The 10-year vision**: Issue trackers become commodity backends. isq is the infrastructure layer—whether accessed directly by humans, or (increasingly) by AI agents acting on behalf of humans.

---

## 2. Where to Play

### Primary Market: Developers and their AI agents

**Who (directly):**
- Professional developers who use terminal as primary interface
- Open source maintainers managing high-volume issue triage
- Developers using AI coding agents (Claude Code, Cursor, Aider)

**Who (through AI agents):**
- Entire teams—PMs, designers, anyone who can talk to an AI agent
- The agent uses isq; the human doesn't need to learn CLI
- Reports, dashboards, planning artifacts generated on demand by agent

**Platform reach:**
- Desktop terminal (primary)
- Claude Code web sandbox (isq installs there—this IS mobile/web)
- Any environment where AI agents run

**We complement, not replace**: isq is infrastructure. Linear, GitHub, Forgejo remain the system of record. isq is how you access them.

### Forge Coverage

| Forge | Priority | Rationale |
|-------|----------|-----------|
| GitHub | P0 | Default for open source, largest market |
| Linear | P0 | Best-in-class UX, proves we can match quality |
| Forgejo/Gitea | P1 | Self-host crowd, GitHub refugees |
| GitLab | P1 | Enterprise + self-host |
| Jira | P2 | Enterprise necessity, large market |

### Stages of Development Loop

isq should support the **entire loop**—directly for some stages, through AI agents for others:

| Stage | How isq Fits |
|-------|--------------|
| Planning | Agent queries isq, generates planning artifacts |
| Decomposition | Agent helps break down; isq creates/links issues |
| Coordination | "What should I work on?" → agent + isq answers |
| **Execution** | Developer + AI in terminal, deep system integration |
| **Capture** | Quick logging without context switch |
| Testing/Refinement | Sub-issues, scope changes, agent-assisted |
| Code Review | PR integration, agent can summarize |
| **Triage** | Process queue—direct CLI or agent-assisted |
| Support | Quick bug logging, agent can gather context |
| Reporting | Agent queries isq, generates reports on demand |

**The shift**: We don't say "use Linear UI for planning." We say "isq provides the data, agent provides the UX." Any stage is accessible.

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

### Guiding Principle

**What's insanely great for humans is usually great for AI agents too.**

We don't design "for agents" vs "for humans." We design for excellence—speed, reliability, composability, deep integration—and both benefit. An insanely great CLI serves:
- Developers who prefer direct control
- Communities not yet bought into the AI agent future
- AI agents that need reliable, fast, structured access
- Debugging when agents make mistakes

### Must Be World-Class At:

| Capability | Why Critical |
|------------|--------------|
| **Speed** | Instant reads, fast writes. Compounds for both humans and agents. |
| **System integration** | Worktrees, branches, commits—understand the dev environment deeply |
| **Reliability** | Daemon just works. Cache is accurate. No babysitting. |
| **Forge abstraction** | Clean trait boundary; adding forges should be easy |
| **Composability** | Small commands that chain well. Great for scripts, agents, humans. |
| **Structured output** | `--json` everywhere. Agents need it. Scripts need it. |

### Must Be Good Enough At:

| Capability | Why |
|------------|-----|
| Multi-repo management | Developers work across repos, need unified view |
| Offline writes | Queue when disconnected, sync when back |
| Error messages | Clear enough for humans AND agents to recover |

### Explicitly Deprioritized:

| Capability | Why Not |
|------------|---------|
| Rich text editing | Use $EDITOR or let agent compose |
| Native dashboards/reports | Agent generates artifacts on demand |
| Native mobile app | Claude Code web sandbox covers this |

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
| Nature | System-level infrastructure | Application with UI |
| Users | Developers + AI agents (whole team via agents) | Developers only |
| Interface | CLI that's great for humans AND agents | Separate human/agent interfaces |
| Architecture | Offline-first, daemon, local state | Online-first, stateless |
| Forge strategy | Universal abstraction | GitHub-only or fork per forge |
| AI strategy | Deep system integration | API wrapper (MCP-only) |
| UX philosophy | Insanely great for humans = great for agents | Design separately for each |
| Pricing | Open source, free | SaaS, freemium |
| Scope | Infrastructure layer for issue access | Full project management |

---

## 7. What Must Be True

For this strategy to succeed:

1. **AI agents become primary interface for dev tools**: This is the bet. Agents need system-level tools, not just APIs.
2. **Terminal/CLI remains where agents run**: Even if humans use chat UI, agents execute in terminal environments.
3. **GitHub continues to degrade**: Platform frustration drives search for alternatives.
4. **We execute on speed & reliability**: If isq isn't dramatically better, there's no reason to adopt.
5. **Forge diversity persists**: If everyone consolidates on one platform, universal abstraction is less valuable.

### Risks

| Risk | Mitigation |
|------|------------|
| GitHub builds great CLI | Focus on multi-forge, offline-first—things GitHub won't do |
| Linear builds system-level tool | Move faster, open source community, forge diversity |
| AI agents get native issue access | Be the layer that provides system context (git, worktrees, local state) |
| Agents don't need system integration | Insanely great CLI still serves humans; we don't lose |

---

## 8. The One-Liner

**isq is the system-level infrastructure for issue tracking—instant, offline-first, universal, and designed for a world where AI agents are the primary interface.**

Not a web UI. Not an API wrapper. Infrastructure that understands your development environment (git, worktrees, local state) and makes issue management a seamless part of coding—whether you're typing commands or talking to an AI agent.

---

## 9. The Asymmetric Bet

If AI agents become the primary way developers interact with tools → isq wins big (system-level integration beats API wrappers)

If AI agents don't take over → isq still wins (insanely great CLI serves humans directly, open source communities adopt it)

We're not betting the farm on AI agents. We're building something great for humans that happens to be even better when AI agents are involved. That's the asymmetry.
