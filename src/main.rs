use clap::Parser;

use wslc_compose::{run, Cli};

fn main() {
    match wslc_compose::run_sdk_daemon_if_requested() {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            eprintln!("SDK daemon error: {error}");
            std::process::exit(1);
        }
    }
    if let Err(error) = run(Cli::parse()) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
