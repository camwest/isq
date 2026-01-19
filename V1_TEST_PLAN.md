# isq V1 Test Plan

**Target Release**: ~Feb 1
**Goal**: Production-ready CLI for daily use without bugs

---

## Pre-Test Setup

```bash
# Build release binary
cargo build --release

# Run lint checks
scripts/lint.sh --ci

# Run all unit tests
cargo test
```

---

## 1. Installation & Updates

### 1.1 Fresh Install

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 1.1.1 | macOS/Linux install | `curl -LsSf https://cameronwestland.com/isq/install.sh \| sh` | Binary installed, PATH configured |
| 1.1.2 | Windows install | `irm https://cameronwestland.com/isq/install.ps1 \| iex` | Binary installed |
| 1.1.3 | Version check | `isq --version` | Version number displayed |
| 1.1.4 | Help available | `isq --help` | All commands listed |

### 1.2 Update Flow

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 1.2.1 | Check for updates | `isq update check` | Shows current vs latest version |
| 1.2.2 | Check JSON output | `isq update check --json` | Valid JSON with version info |
| 1.2.3 | Install update | `isq update install` | Downloads and installs new version |

### 1.3 Uninstall

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 1.3.1 | Dry run | `isq uninstall --dry-run` | Shows what would be removed |
| 1.3.2 | Keep config | `isq uninstall --keep-config` | Removes binary/cache, keeps config |
| 1.3.3 | Full uninstall | `isq uninstall -y` | Removes all isq files |

---

## 2. Authentication

### 2.1 GitHub

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 2.1.1 | OAuth login | `isq link github` (new user) | Opens browser, completes OAuth |
| 2.1.2 | PAT fallback | `isq link github` (no browser) | Prompts for PAT |
| 2.1.3 | gh CLI token reuse | Have `gh` auth'd, run `isq link github` | Detects and offers to use gh token |
| 2.1.4 | Logout | `isq logout github` | Removes stored credentials |
| 2.1.5 | Re-auth | `isq link github` after logout | Can re-authenticate |

### 2.2 Linear

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 2.2.1 | OAuth PKCE | `isq link linear` | Opens browser, completes OAuth |
| 2.2.2 | API key auth | `isq link linear` (select API key) | Prompts for API key |
| 2.2.3 | Logout | `isq logout linear` | Removes stored credentials |

### 2.3 JIRA Cloud

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 2.3.1 | OAuth PKCE | `isq link jira` | Opens browser, completes OAuth |
| 2.3.2 | API token auth | `isq link jira` (select API token) | Prompts for email + token |
| 2.3.3 | List projects | `isq link jira -o list-projects` | Shows available projects |
| 2.3.4 | Link project | `isq link jira -o project=MYPROJ` | Links to specific project |
| 2.3.5 | Logout | `isq logout jira` | Removes stored credentials |

---

## 3. Repo Linking

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 3.1 | Link repo | `isq link github` (in git repo) | Repo linked, commit hook installed |
| 3.2 | Status check | `isq status` | Shows linked forge, auth status, sync health |
| 3.3 | Unlink | `isq unlink` | Removes link and commit hook |
| 3.4 | Link non-repo | `isq link github` (not in git repo) | Clear error message |
| 3.5 | Multiple forges | Link GitHub in repo1, Linear in repo2, verify independent | Each repo maintains separate link |

---

## 4. Issue Operations

### 4.1 Listing Issues

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 4.1.1 | Basic list | `isq issue list` | Shows issues (instant from cache) |
| 4.1.2 | Filter by state | `isq issue list --state=open` | Only open issues |
| 4.1.3 | Filter by state closed | `isq issue list --state=closed` | Only closed issues |
| 4.1.4 | Filter by label | `isq issue list --label=bug` | Only issues with "bug" label |
| 4.1.5 | Multiple labels | `isq issue list --label=bug --label=p0` | Issues with both labels |
| 4.1.6 | My issues | `isq issue list --mine` | Only issues assigned to me |
| 4.1.7 | Specific IDs | `isq issue list --id 7,12,45` | Only issues #7, #12, #45 |
| 4.1.8 | Sort by priority | `isq issue list --sort priority` | Priority-ordered (default) |
| 4.1.9 | Sort by newest | `isq issue list --sort newest` | Newest first |
| 4.1.10 | Sort by oldest | `isq issue list --sort oldest` | Oldest first |
| 4.1.11 | Sort by updated | `isq issue list --sort updated` | Most recently updated first |
| 4.1.12 | JSON output | `isq issue list --json` | Valid parseable JSON |

### 4.2 Issue Hierarchy

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 4.2.1 | Default (root only) | `isq issue list` (when sub-issues exist) | Shows only root issues |
| 4.2.2 | Tree view | `isq issue list --tree` | Shows hierarchy with indentation |
| 4.2.3 | Flat view | `isq issue list --flat` | Shows all issues including sub-issues |
| 4.2.4 | Children of | `isq issue list --children-of 42` | Shows children of issue #42 |

### 4.3 Show Issue

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 4.3.1 | Show issue | `isq issue show 123` | Displays issue details |
| 4.3.2 | Show JSON | `isq issue show 123 --json` | Valid JSON output |
| 4.3.3 | Non-existent issue | `isq issue show 999999` | Clear error message |

### 4.4 Create Issue

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 4.4.1 | Basic create | `isq issue create --title "Test issue"` | Issue created, ID returned |
| 4.4.2 | With body | `isq issue create --title "Test" --body "Description"` | Issue with body created |
| 4.4.3 | With label | `isq issue create --title "Bug" --label bug` | Issue with label created |
| 4.4.4 | With parent | `isq issue create --title "Subtask" --parent 42` | Sub-issue created |
| 4.4.5 | Pipe content | `echo "Body text" \| isq issue create --title "Test"` | Body from stdin |
| 4.4.6 | JSON output | `isq issue create --title "Test" --json` | Returns JSON with issue data |

### 4.5 Comment

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 4.5.1 | Add comment | `isq issue comment 123 "Test comment"` | Comment added |
| 4.5.2 | Pipe comment | `echo "Comment text" \| isq issue comment 123` | Comment from stdin |
| 4.5.3 | JSON output | `isq issue comment 123 "Test" --json` | Returns JSON with comment data |

### 4.6 State Changes

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 4.6.1 | Close issue | `isq issue close 123` | Issue closed |
| 4.6.2 | Reopen issue | `isq issue reopen 123` | Issue reopened |
| 4.6.3 | Close JSON | `isq issue close 123 --json` | Returns JSON |

### 4.7 Labels

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 4.7.1 | Add label | `isq issue label 123 add bug` | Label added |
| 4.7.2 | Remove label | `isq issue label 123 remove bug` | Label removed |
| 4.7.3 | Non-existent label | `isq issue label 123 add nonexistent` | Clear error message |

### 4.8 Assignment

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 4.8.1 | Assign user | `isq issue assign 123 username` | Issue assigned |
| 4.8.2 | Unassign | `isq issue assign 123 ""` | Issue unassigned |

---

## 5. Labels

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 5.1 | List labels | `isq label list` | Shows all repo labels |
| 5.2 | List JSON | `isq label list --json` | Valid JSON output |
| 5.3 | Create label | `isq label create test-label --color ff0000` | Label created |
| 5.4 | Create with desc | `isq label create test-label --description "Test"` | Label with description |

---

## 6. Goals (Milestones/Projects)

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 6.1 | List goals | `isq goal list` | Shows milestones/projects |
| 6.2 | List open | `isq goal list --state=open` | Only open goals |
| 6.3 | Show goal | `isq goal show "Q1 Release"` | Goal details |
| 6.4 | Create goal | `isq goal create "Q2 Release" --target "2024-06-30"` | Goal created |
| 6.5 | Assign issue | `isq goal assign 123 "Q2 Release"` | Issue assigned to goal |
| 6.6 | Close goal | `isq goal close "Q1 Release"` | Goal closed |
| 6.7 | JSON output | `isq goal list --json` | Valid JSON |

---

## 7. Views (Saved Filters)

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 7.1 | Create view | `isq view create my-bugs --label=bug --state=open --mine` | View saved |
| 7.2 | List views | `isq view list` | Shows all saved views |
| 7.3 | Use view | `isq issue list @my-bugs` | Applies saved filters |
| 7.4 | View + override | `isq issue list @my-bugs --state=closed` | Override view settings |
| 7.5 | Create hierarchy view | `isq view create epics --root-only` | View saved |
| 7.6 | Create tree view | `isq view create hierarchy --tree` | View saved |
| 7.7 | Delete view | `isq view delete my-bugs` | View removed |
| 7.8 | View JSON | `isq view list --json` | Valid JSON |

---

## 8. Development Workflow (Worktrees)

### 8.1 Start Flow

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 8.1.1 | Start issue | `isq start 123` | Worktree created, branch created |
| 8.1.2 | Branch naming | Check branch name after start | Format: `123-slugified-title` |
| 8.1.3 | Setup script | `.config/isq.toml` with setup command | Setup script executes |
| 8.1.4 | on_start labels | Config with `add_labels = ["in progress"]` | Label added on start |
| 8.1.5 | on_start assign | Config with `assign_self = true` | Assigned to current user |

### 8.2 Current Issue

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 8.2.1 | Show current | `isq` (in worktree) | Shows current issue + context |
| 8.2.2 | Current ID | `isq current` | Prints just the issue ID |
| 8.2.3 | Current quiet | `isq current --quiet` | Prints ID, no error if none |
| 8.2.4 | No current | `isq current` (not in worktree) | Clear error message |

### 8.3 Cleanup Flow

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 8.3.1 | Cleanup | `isq cleanup` | Worktree removed, association cleared |
| 8.3.2 | Cleanup keep | `isq cleanup --keep` | Preserves worktree directory |
| 8.3.3 | on_cleanup labels | Config with `remove_labels = ["in progress"]` | Label removed |
| 8.3.4 | on_cleanup transition | Linear/JIRA config with `transition = "backlog"` | Issue transitioned |

---

## 9. Sync & Daemon

### 9.1 Manual Sync

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 9.1.1 | Manual sync | `isq sync` | Issues synced from forge |
| 9.1.2 | Sync quiet | `isq sync --quiet` | No output on success |
| 9.1.3 | Sync progress | `isq sync` | Shows sync progress |

### 9.2 Daemon Control

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 9.2.1 | Daemon status | `isq daemon status` | Shows running state, watched repos |
| 9.2.2 | Daemon start | `isq daemon start` | Daemon starts |
| 9.2.3 | Daemon stop | `isq daemon stop` | Daemon stops gracefully |
| 9.2.4 | Daemon logs | `isq daemon logs -n 50` | Shows recent logs |
| 9.2.5 | Daemon logs follow | `isq daemon logs -f` | Tails logs |
| 9.2.6 | Watch repo | `isq daemon watch` | Repo added to watch list |
| 9.2.7 | Unwatch repo | `isq daemon unwatch` | Repo removed from watch list |
| 9.2.8 | Single instance | Start daemon twice | Second attempt recognizes existing |

---

## 10. Diagnostics

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 10.1 | Status overview | `isq status` | Shows auth, daemon, sync health |
| 10.2 | Doctor all | `isq doctor` | Runs all diagnostic checks |
| 10.3 | Doctor auth | `isq doctor --check=auth` | Checks authentication |
| 10.4 | Doctor repo | `isq doctor --check=repo` | Checks repo link |
| 10.5 | Doctor sync | `isq doctor --check=sync` | Checks sync state |
| 10.6 | Doctor database | `isq doctor --check=database` | Checks SQLite integrity |
| 10.7 | Doctor network | `isq doctor --check=network` | Checks API connectivity |
| 10.8 | Doctor service | `isq doctor --check=service` | Checks daemon status |
| 10.9 | Doctor install | `isq doctor --check=install` | Checks installation |

---

## 11. Offline Behavior

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 11.1 | Read offline | Disconnect network, `isq issue list` | Works from cache |
| 11.2 | Write offline | Disconnect, `isq issue comment 123 "Test"` | Queued locally |
| 11.3 | Sync on reconnect | Reconnect, `isq sync` | Pending ops applied |
| 11.4 | Offline status | `isq status` (offline) | Shows offline state |

---

## 12. Cross-Forge Consistency

Run these tests on each forge (GitHub, Linear, JIRA):

| # | Test | Command | GitHub | Linear | JIRA |
|---|------|---------|--------|--------|------|
| 12.1 | Link | `isq link <forge>` | [ ] | [ ] | [ ] |
| 12.2 | List issues | `isq issue list` | [ ] | [ ] | [ ] |
| 12.3 | Create issue | `isq issue create --title "Test"` | [ ] | [ ] | [ ] |
| 12.4 | Add comment | `isq issue comment <id> "Test"` | [ ] | [ ] | [ ] |
| 12.5 | Close issue | `isq issue close <id>` | [ ] | [ ] | [ ] |
| 12.6 | Reopen issue | `isq issue reopen <id>` | [ ] | [ ] | [ ] |
| 12.7 | Add label | `isq issue label <id> add <label>` | [ ] | [ ] | [ ] |
| 12.8 | Assign | `isq issue assign <id> <user>` | [ ] | [ ] | [ ] |
| 12.9 | Goals | `isq goal list` | [ ] | [ ] | [ ] |
| 12.10 | Priority sort | `isq issue list --sort priority` | [ ] | [ ] | [ ] |
| 12.11 | Hierarchy | `isq issue list --tree` | [ ] | [ ] | [ ] |
| 12.12 | JSON output | `isq issue list --json` | [ ] | [ ] | [ ] |
| 12.13 | Worktree start | `isq start <id>` | [ ] | [ ] | [ ] |
| 12.14 | Cleanup | `isq cleanup` | [ ] | [ ] | [ ] |

---

## 13. JSON Output Validation

| # | Test | Command | Validation |
|---|------|---------|------------|
| 13.1 | Issue list | `isq issue list --json \| jq .` | Valid JSON, array of issues |
| 13.2 | Issue show | `isq issue show 123 --json \| jq .` | Valid JSON, issue object |
| 13.3 | Issue create | `isq issue create --title "Test" --json \| jq .id` | Returns created issue ID |
| 13.4 | Comment | `isq issue comment 123 "Test" --json \| jq .` | Valid JSON, comment object |
| 13.5 | Goal list | `isq goal list --json \| jq .` | Valid JSON, array |
| 13.6 | View list | `isq view list --json \| jq .` | Valid JSON, array |
| 13.7 | Status | `isq status --json \| jq .` | Valid JSON |
| 13.8 | Update check | `isq update check --json \| jq .` | Valid JSON with versions |

---

## 14. Error Handling

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 14.1 | No link | `isq issue list` (unlinked repo) | "Run `isq link` first" |
| 14.2 | Invalid issue | `isq issue show 999999999` | "Issue not found" |
| 14.3 | Invalid label | `isq issue label 123 add nonexistent` | Clear error message |
| 14.4 | Network error | Disconnect during sync | Graceful error, can retry |
| 14.5 | Auth expired | Invalidate token, run command | Prompts to re-auth |
| 14.6 | Rate limited | Trigger rate limit | Backs off, informs user |
| 14.7 | Invalid view | `isq issue list @nonexistent` | "View not found" |

---

## 15. Performance

| # | Test | Command | Target |
|---|------|---------|--------|
| 15.1 | Cold list | `time isq issue list` (first run after sync) | <100ms |
| 15.2 | Warm list | `time isq issue list` (subsequent) | <50ms |
| 15.3 | Large repo | List 1000+ issues | <200ms |
| 15.4 | JSON output | `time isq issue list --json` | <100ms |

---

## 16. Configuration

### 16.1 Per-Repo Config (`.config/isq.toml`)

| # | Test | Config | Expected |
|---|------|--------|----------|
| 16.1.1 | Worktree setup | `[worktree] setup = "npm install"` | Script runs on `isq start` |
| 16.1.2 | on_start labels | `[on_start] add_labels = ["wip"]` | Label added |
| 16.1.3 | on_start assign | `[on_start] assign_self = true` | Self-assigned |
| 16.1.4 | on_cleanup labels | `[on_cleanup] remove_labels = ["wip"]` | Label removed |
| 16.1.5 | Priority mapping | `[priority] P0 = 0` | GitHub labels mapped to priority |
| 16.1.6 | Invalid config | Malformed TOML | Clear error message |

### 16.2 User Config (`~/.config/isq/config.toml`)

| # | Test | Config | Expected |
|---|------|--------|----------|
| 16.2.1 | Default JSON | `[defaults] json = true` | All commands output JSON |
| 16.2.2 | Saved views | `[views.my-bugs]` | View accessible via @my-bugs |

---

## 17. JIRA-Specific Tests

| # | Test | Command / Steps | Expected |
|---|------|-----------------|----------|
| 17.1 | List fields | `isq forge jira list-fields` | Shows available JIRA fields |
| 17.2 | Custom fields | Create issue with custom field | Field populated correctly |
| 17.3 | ADF conversion | Create issue with markdown body | Converted to JIRA format |
| 17.4 | Workflow transitions | `isq start` with JIRA transition config | Issue transitioned |

---

## 18. Security

| # | Test | Steps | Expected |
|---|------|-------|----------|
| 18.1 | Credentials stored | Check credential storage location | Encrypted/protected file |
| 18.2 | No secrets in logs | Run commands, check daemon logs | No tokens/passwords visible |
| 18.3 | OAuth PKCE | Monitor auth flow | PKCE code verifier used |

---

## 19. Edge Cases

| # | Test | Steps | Expected |
|---|------|-------|----------|
| 19.1 | Unicode in title | Create issue with emoji/unicode title | Handled correctly |
| 19.2 | Long title | Create issue with 500+ char title | Truncated or error |
| 19.3 | Empty body | `isq issue create --title "Test" --body ""` | Works correctly |
| 19.4 | Special chars | Create issue with `<script>` in title | Escaped/safe |
| 19.5 | Concurrent ops | Multiple `isq` commands simultaneously | No data corruption |
| 19.6 | Large comment | `isq issue comment <id>` with 10KB text | Handled correctly |

---

## Sign-Off Checklist

### Build & Lint
- [ ] `cargo build --release` passes
- [ ] `cargo test` all tests pass
- [ ] `scripts/lint.sh --ci` passes
- [ ] No compiler warnings

### Core Functionality
- [ ] GitHub authentication works
- [ ] Linear authentication works
- [ ] JIRA authentication works
- [ ] Issue CRUD operations work on all forges
- [ ] Sync works reliably
- [ ] Daemon runs stably
- [ ] Offline mode works

### User Experience
- [ ] Help text is clear (`--help` on all commands)
- [ ] Error messages guide next steps
- [ ] Performance targets met
- [ ] JSON output valid on all commands

### Release Artifacts
- [ ] Install scripts tested
- [ ] Binary checksums generated
- [ ] GitHub Release created
- [ ] Documentation current

---

**Tester**: ___________________
**Date**: ___________________
**Version**: ___________________
**Result**: [ ] PASS / [ ] FAIL
