# isq Development Guidelines

## Forge Abstraction

All forge-specific code (GitHub, Linear, etc.) must stay in its respective module (`github.rs`, `linear.rs`).

- Common types like `Issue` belong in `forge.rs`
- Never import forge-specific types (e.g., `crate::github::Issue`) outside the forge modules
- Forge modules should convert their API responses to the common `forge::Issue` type internally
