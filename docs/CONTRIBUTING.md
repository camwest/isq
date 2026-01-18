# Contributing

## Creating Issues

Use `isq issue create` with problem-framing titles (what's wrong, not the solution).

```
isq issue create --title "Cannot assign issues to goals during creation" --body "**Problem**: ...
**Goal**: ...
**Success criteria**:
- ..."
```

Body format:
- **Problem**: 1-2 sentences
- **Goal**: 1 sentence
- **Success criteria**: bullet list

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org/). CI enforces this on PRs.

Format: `type(scope): description`

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`

Examples:
```
feat(cli): add issue search command
fix(sync): handle rate limit errors gracefully
docs: update installation instructions
refactor(db): extract query builders
```

Rules:
- Subject must be lowercase ("add feature" not "Add feature")
- Max 72 characters in the header
- Use imperative mood ("add" not "added" or "adds")
