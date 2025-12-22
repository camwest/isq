# isq

A CLI for GitHub/GitLab/Forgejo issues. Instant. Offline-first. AI-agent native.

> "isq" = issue queue (if anyone asks)

## Problem

GitHub's UX is rotting. Zig's migration post cited "sluggish and broken" features and unwanted Copilot pressure. The platform is slow, mouse-dependent, cluttered.

Linear proved the fix: fast, keyboard-driven, detail-obsessed. But Linear caps free tier at ~250 issues—unusable for OSS.

The gap: Linear-quality speed for open source issue management.

## Insight

CLI-first isn't a constraint. It's the feature.

1. **Humans**: Power users live in the terminal
2. **AI agents**: Claude Code, Cursor, Aider work via CLI—they can't click GUIs

A fast CLI with local cache serves both.

## Example: Claude Code Integration

```
User: "Find all bugs related to auth and add them to milestone 2.0"

Claude Code:
$ isq issue list --label=bug --query="auth" --json
$ isq issue update 423 --milestone="2.0"
$ isq issue update 419 --milestone="2.0"
$ isq issue update 415 --milestone="2.0"
Done. Added 3 auth-related bugs to milestone 2.0.
```

```
User: "Close issue 423 with a comment that it's fixed in the latest release"

Claude Code:
$ isq issue comment 423 "Fixed in v2.1.0 release"
$ isq issue close 423
Done.
```

GitHub's web UI can't do this. Linear can't do this. This is the moat.

## Solution

A CLI that gives instant responses by reading from local cache, syncing in background.

Same issues, labels, milestones as GitHub. Different interface.

isq doesn't replace GitHub. It replaces the experience of using GitHub.

## Business Model

The lazygit model:
- Free, MIT license
- 69.5k stars, mass adoption
- GitHub Sponsors (individuals)
- Commercial sponsors (Warp, Tuple—companies wanting dev goodwill)

No SaaS. No enterprise tier. Beloved tool sustained by community.

## MVP Scope (v0.1)

**CLI commands:**
```bash
isq issue list [--label=X] [--state=open|closed] [--json]
isq issue show <id> [--json]
isq issue create --title="X" [--body="Y"] [--label=Z]
isq issue comment <id> "message"
isq issue label <id> <add|remove> <label>
isq issue assign <id> <user>
isq issue close <id>
isq issue reopen <id>
isq sync
```

**Core features:**
- Local SQLite cache (instant reads)
- Background sync (cache stays fresh)
- Offline queue (write offline, sync later)
- JSON output (AI agent friendly)

**Backends:**
- GitHub
- GitLab
- Forgejo

## Future (v0.2+)

- TUI layer (ratatui) for interactive use
- Milestones
- Multiple repos
- Notifications
- PRs (list, view, comment, review, merge)
- CI status

## Out of Scope

- Code editing
- CI configuration
- Repo settings
- Wiki/Discussions

## References

- [lazygit](https://github.com/jesseduffield/lazygit) - 69.5k stars, the model
- [uv](https://github.com/astral-sh/uv) - Rust CLI, speed obsession inspiration
- [git-bug](https://github.com/git-bug/git-bug) - distributed issues in git, prior art
- [Zig migration post](https://ziglang.org/news/migrating-from-github-to-codeberg/) - articulates GitHub's decay
- [Linear](https://linear.app) - the UX standard to match
