# Roadmap: isq as "Issue Tracker Email Client"

**Vision:** isq defines a new category—"issue tracker as email client"—and becomes the default way developers interact with issues.

**Inspirations:**
- **Linear:** Speed obsession, keyboard-first, opinionated workflows, cycles, inbox
- **jj:** Reimagine the paradigm, not just wrap existing tools (anonymous branches, undo, no staging)
- **Superhuman/Hey:** Email as workflow, not archive. Inbox zero. Snooze. Triage.
- **lazygit:** TUI done right. Fast, intuitive, keyboard-driven.

---

## Core Insight

Issues ARE emails:
- They **arrive** (assigned to you, mentioned, subscribed)
- They need **triage** (urgent? delegate? snooze? close?)
- They have **state** (unread, in progress, waiting, done)
- They have **threads** (comments)
- They can be **organized** (labels, views, filters)

But unlike email clients, issue trackers force you into THEIR workflow. isq flips this—YOUR workflow, THEIR data.

---

## Phase 1: Personal Inbox Model

**The killer feature.** No issue tracker does this well.

### `isq inbox` - Your Personal Issue Stream

```bash
# Everything that needs YOUR attention
$ isq inbox

INBOX (12 items)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  NEW   #892  Auth flow broken on Safari       @you   2h ago
  NEW   #891  Add dark mode toggle             @you   5h ago
  ●     #887  Perf regression in dashboard     @you   1d ago
        #845  Refactor user service            @you   3d ago

WAITING (3 items)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  ↻     #823  Need API design review           created by you   2d ago
  ↻     #801  Blocked on infra team            created by you   5d ago

SNOOZED (2 items)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  ⏸     #756  Revisit caching strategy         until Mon
  ⏸     #702  Tech debt cleanup                until next sprint
```

### Inbox Categories

| Category | Contents |
|----------|----------|
| **Inbox** | Assigned to me, mentioned in, subscribed with updates |
| **Waiting** | My issues where I'm waiting for others |
| **Snoozed** | Hidden until date/event |
| **Done** | Archived (closed or manually marked done) |

### Local State (Email-Like)

```sql
-- New table: personal state per issue
CREATE TABLE issue_state (
    repo TEXT NOT NULL,
    issue_number INTEGER NOT NULL,
    seen_at TEXT,           -- last time you viewed it
    snoozed_until TEXT,     -- date or NULL
    snooze_trigger TEXT,    -- 'date' | 'next_comment' | 'next_mention'
    archived BOOLEAN,       -- manually marked done
    notes TEXT,             -- personal notes (local only)
    PRIMARY KEY (repo, issue_number)
);
```

### Commands

```bash
isq inbox                     # show your inbox
isq inbox --all               # include snoozed/archived

isq triage <id>               # interactive triage (assign, label, snooze, close)
isq snooze <id> tomorrow      # snooze until tomorrow
isq snooze <id> monday        # snooze until next Monday
isq snooze <id> --until-comment  # snooze until someone comments
isq done <id>                 # archive (you're done, regardless of open/closed)
isq note <id> "remember to..."   # add personal note
```

---

## Phase 2: TUI (Terminal UI)

**Like lazygit, but for issues.**

```bash
$ isq ui

┌─ INBOX ─────────────────────────────────────────────────────┐
│ ● #892  Auth flow broken on Safari             @you    2h  │
│   #891  Add dark mode toggle                   @you    5h  │
│   #887  Perf regression in dashboard           @you    1d  │
│ ▸ #845  Refactor user service                  @you    3d  │
├─────────────────────────────────────────────────────────────┤
│ Refactor user service                               bug p2 │
│                                                             │
│ The user service has grown too large. We should split it   │
│ into:                                                       │
│ - UserAuthService                                           │
│ - UserProfileService                                        │
│ - UserPreferencesService                                    │
│                                                             │
│ ─── @alice (3d ago) ────────────────────────────────────── │
│ I can take the auth portion. @bob thoughts on the split?   │
│                                                             │
│ ─── @bob (2d ago) ──────────────────────────────────────── │
│ LGTM. Let's use the adapter pattern for migration.         │
├─────────────────────────────────────────────────────────────┤
│ j/k:move  o:open  c:comment  a:assign  l:label  s:snooze   │
│ d:done    x:close  /:search  ?:help                        │
└─────────────────────────────────────────────────────────────┘
```

### Key Bindings (Vim-Inspired)

| Key | Action |
|-----|--------|
| `j/k` | Move up/down |
| `g/G` | Top/bottom |
| `o` | Open in browser |
| `Enter` | Expand/collapse |
| `c` | Comment |
| `a` | Assign |
| `l` | Add label |
| `L` | Remove label |
| `s` | Snooze |
| `d` | Mark done |
| `x` | Close issue |
| `r` | Reopen issue |
| `/` | Search |
| `v` | Switch view |
| `?` | Help |
| `q` | Quit |

### Views

```
1: Inbox      2: All Open    3: Created    4: Assigned    5: Custom...
```

---

## Phase 3: Views & Smart Filters

**Saved queries, like Gmail labels but better.**

### Default Views

```bash
$ isq view list

VIEWS
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  inbox       Issues needing your attention (default)
  assigned    All issues assigned to you
  created     Issues you created
  watching    Subscribed issues
  all         All open issues
```

### Custom Views

```bash
# Create a view
$ isq view create "p0-bugs" --filter 'label:bug label:p0 state:open'
✓ Created view "p0-bugs"

# Use it
$ isq issue list --view p0-bugs
$ isq p0-bugs   # shorthand (alias)

# Edit
$ isq view edit p0-bugs --filter 'label:bug label:p0,p1 state:open'
```

### Query Language

```
# Simple filters (current)
state:open label:bug assignee:@me

# Compound filters
(label:bug OR label:regression) AND state:open

# Time-based
created:>7d              # created in last 7 days
updated:<30d             # not updated in 30 days
commented-by:@alice      # alice has commented

# Personal state
seen:false               # haven't viewed
snoozed:true             # currently snoozed

# Text search
title:"login bug"        # title contains
body:"workaround"        # body contains
"auth flow"              # full-text search
```

### Smart Views (Auto-Generated)

```bash
$ isq view suggest

SUGGESTED VIEWS (based on your activity)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  stale-assigned    Issues assigned to you, no activity 7d+
  my-p0s            P0 bugs you're involved in
  review-needed     PRs/issues waiting on your review
```

---

## Phase 4: Workflow Enhancements

### Quick Create from Branch

```bash
# You're on a branch, create issue for it
$ isq issue create --from-branch
✓ Created #501: feat/add-dark-mode → "Add dark mode support"
```

### Branch from Issue

```bash
$ isq branch 423
✓ Created branch: fix/423-login-bug
✓ Linked issue #423

# When you open a PR, it auto-links
```

### Templates

```bash
$ isq template list
  bug          Bug report template
  feature      Feature request template
  task         Simple task

$ isq issue create --template bug
# Opens $EDITOR with template, or inline prompts

$ isq template create security-issue
# Save commonly used templates
```

### Quick Transitions

```bash
$ isq start 423      # Assign to self, add 'in-progress' label
$ isq review 423     # Add 'needs-review' label
$ isq ship 423       # Close, add 'shipped' label, comment with PR link
```

---

## Phase 5: AI Enhancement

**Go beyond "AI-agent native" to "AI-enhanced."**

### Summarize

```bash
$ isq issue show 892 --summarize

#892: Auth flow broken on Safari
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

SUMMARY (47 comments → 3 key points):

1. Safari blocks third-party cookies by default since v16.4
2. Current OAuth flow relies on cookie for state parameter
3. Proposed fix: Use localStorage + postMessage instead

BLOCKERS:
- Waiting on security team review (mentioned by @alice 2d ago)

NEXT STEPS:
- Implement localStorage approach (assigned to @bob)
- Update OAuth documentation
```

### Smart Triage

```bash
$ isq triage --suggest

TRIAGE SUGGESTIONS
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#893 "App crashes on startup"
  → Suggest: label:bug label:p0 (crash = high priority)
  → Suggest: assign @oncall (no assignee, mentions crash)

#894 "Would be nice to have dark mode"
  → Suggest: label:enhancement label:p3
  → Suggest: snooze 30d (nice-to-have, low urgency)

Apply suggestions? [y/n/select]
```

### Semantic Search

```bash
# Find issues about auth, even if they don't say "auth"
$ isq search "authentication problems"

RESULTS (semantic match)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  #892  Auth flow broken on Safari         (exact: "auth")
  #756  Login fails after password reset   (semantic: auth-related)
  #701  Session expires unexpectedly       (semantic: auth-related)
  #445  "Remember me" doesn't work         (semantic: auth-related)
```

### Auto-Label on Create

```bash
$ isq issue create --title "Crash when clicking submit button"
  ✓ Created #895
  ✓ Auto-labeled: bug, needs-triage (detected: "crash")
```

---

## Phase 6: Multi-Forge Excellence

### Unified Inbox Across Forges

```bash
# Link multiple repos
$ isq link github        # work-frontend (GitHub)
$ isq link linear        # work-backend (Linear)
$ isq link forgejo       # personal-site (self-hosted)

# One inbox to rule them all
$ isq inbox --all-repos

INBOX
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  GH  work-frontend   #423  Fix dark mode toggle
  LN  work-backend    ISQ-42  API rate limiting
  FG  personal-site   #12   Update about page
```

### Cross-Forge Search

```bash
$ isq search "auth" --all-repos
```

### Planned Forges

| Forge | Priority | Notes |
|-------|----------|-------|
| GitHub | ✅ Done | Full support |
| Linear | ✅ Done | Full support |
| GitLab | High | Popular, similar API |
| Forgejo/Gitea | High | Self-host crowd |
| Jira | Medium | Enterprise necessity |
| Plane | Low | Open source Linear |
| Shortcut | Low | Popular in some orgs |

---

## Phase 7: Notifications & Real-Time

### Desktop Notifications

```bash
$ isq config set notifications.enabled true
$ isq config set notifications.filter "assignee:@me OR mentioned:@me"
```

When daemon syncs and finds matching new items → system notification.

### Watch/Unwatch

```bash
$ isq watch 423           # subscribe to updates
$ isq unwatch 423         # unsubscribe
$ isq mute 423            # unsubscribe + hide from inbox
```

### Webhook Support (Future)

For real-time updates instead of polling:

```bash
$ isq webhook setup       # configure GitHub webhook → local daemon
```

---

## Phase 8: Team & Collaboration

### Shared Views

```bash
# Export view for team
$ isq view export "sprint-42" > sprint-42.isqview

# Team member imports
$ isq view import sprint-42.isqview
```

### `.isq/` Directory (Repo-Level Config)

```
.isq/
├── views/           # shared views
│   ├── bugs.isqview
│   └── sprint.isqview
├── templates/       # issue templates
│   ├── bug.md
│   └── feature.md
└── config.toml      # repo-specific settings
```

---

## Phase 9: Distribution & Adoption

### One-Line Install

```bash
curl -LsSf https://isq.dev/install.sh | sh
```

### Package Managers

| Platform | Package |
|----------|---------|
| macOS | `brew install isq` |
| Arch | `yay -S isq` |
| Nix | `nix-env -i isq` |
| Cargo | `cargo install isq` |
| Windows | `winget install isq` / `scoop install isq` |

### Auto-Updates

```bash
$ isq update           # check and update
$ isq config set auto-update true   # auto-update on daemon start
```

---

## Success Metrics

### Must-Haves for Category Definition

1. **10x faster than web UI** - Already there (sub-ms reads)
2. **Works offline** - Already there
3. **Personal inbox model** - Phase 1 (killer feature)
4. **TUI** - Phase 2 (stickiness)
5. **Unified multi-forge** - Phase 6 (moat)

### Adoption Goals

| Milestone | Target |
|-----------|--------|
| GitHub stars | 5,000 |
| Weekly active users | 10,000 |
| Forge integrations | 5+ |
| "Inbox zero" success rate | 80% of users |

---

## Competitive Moat

1. **Offline-first architecture** - Hard to replicate, fundamental advantage
2. **Forge abstraction** - One tool for all trackers
3. **Personal state layer** - Inbox/snooze/archive on top of ANY tracker
4. **AI-native from day 1** - Not bolted on
5. **Speed obsession** - Rust + SQLite, no Electron, no web

---

## Implementation Priority

### Now (v0.2)
- [ ] `isq inbox` command with basic personal state
- [ ] `isq snooze` and `isq done` commands
- [ ] Seen/unseen tracking
- [ ] Personal notes

### Next (v0.3)
- [ ] TUI with ratatui
- [ ] Basic keyboard navigation
- [ ] View switching in TUI

### Later (v0.4+)
- [ ] Custom views with query language
- [ ] AI summarization
- [ ] GitLab/Forgejo backends
- [ ] Desktop notifications
- [ ] Shared views

---

## Non-Goals (Maintained from MVP)

- Replace web UI entirely (co-exist)
- Enterprise features (SSO, audit logs)
- Real-time sync (polling is fine)
- Code editing or CI configuration
- Become a project management tool (stay focused on issues)

---

## Open Questions

1. **Snooze persistence** - Store locally only, or sync to forge as label/field?
2. **TUI framework** - ratatui vs other options?
3. **AI integration** - Local models vs API calls? Privacy concerns?
4. **Query language** - Custom DSL vs JMESPath/JSONPath?
5. **Team features** - How far to go before becoming "project management"?

---

*"Make something people want." — YC*

*"The best issue client is the one you actually use." — isq*
