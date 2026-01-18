//! Update checking and application logic using GitHub Releases API

mod apply;
mod background;
mod check;
mod staged;
mod version;

#[cfg(test)]
mod tests;

// Re-exports for the public API - types are used via type inference externally
#[allow(unused_imports)]
pub use apply::{UpdateResult, apply_update};
pub use background::maybe_check_for_updates_background;
#[allow(unused_imports)]
pub use check::{UpdateInfo, check_for_updates};
#[allow(unused_imports)]
pub use staged::{
    StagedUpdate, apply_staged_update, check_staged_update, cleanup_staged_update, restart_self,
};
pub use version::is_binary_updated;
