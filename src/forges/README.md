# Adding a New Forge

Files to modify: 2 (1 new + 1 existing)

## Steps

1. Create `src/forges/{name}.rs` with:
   - `AUTH: AuthConfig` - keyring service, env var, display name
   - `oauth_flow()` - returns TokenResponse
   - `link(repo_path, args)` - returns LinkResult
   - `{Name}Client` implementing `Forge` trait

2. Update `src/forges/mod.rs`:
   - Add module: `pub mod {name}`
   - Add variant: `ForgeType::{Name}`
   - Add to `ALL_FORGE_TYPES`
   - Add match arms in `ForgeType` methods

main.rs: no changes required.

See `github.rs` and `linear.rs` for examples.
