mod app;
mod backend;
mod cli;
mod config;
mod env;
mod error;
mod model;
mod plan;
mod sdk_daemon;

pub use app::run;
pub use cli::Cli;
pub use error::{Error, Result};

/// Starts the private SDK daemon when this executable was spawned for it.
pub fn run_sdk_daemon_if_requested() -> Result<bool> {
    sdk_daemon::run_if_requested()
}
