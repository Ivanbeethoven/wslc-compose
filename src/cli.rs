use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "wslc-compose",
    version,
    about = "Docker Compose compatible orchestration for Microsoft WSL Containers",
    propagate_version = true
)]
pub struct Cli {
    /// Compose configuration files. Later files override earlier files.
    #[arg(short = 'f', long = "file", action = ArgAction::Append)]
    pub files: Vec<PathBuf>,

    /// Project name used for resources and Compose labels.
    #[arg(short = 'p', long = "project-name", env = "COMPOSE_PROJECT_NAME")]
    pub project_name: Option<String>,

    /// Directory used to resolve relative paths.
    #[arg(long)]
    pub project_directory: Option<PathBuf>,

    /// Alternative environment file used for interpolation.
    #[arg(long)]
    pub env_file: Option<PathBuf>,

    /// Enable one or more Compose profiles.
    #[arg(long, action = ArgAction::Append, env = "COMPOSE_PROFILES", value_delimiter = ',')]
    pub profile: Vec<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Parse, resolve, and render the Compose configuration.
    Config {
        #[arg(short, long)]
        quiet: bool,
        #[arg(long)]
        services: bool,
        #[arg(long)]
        profiles: bool,
        #[arg(long, value_enum, default_value_t = OutputFormat::Yaml)]
        format: OutputFormat,
    },

    /// Pull service images.
    Pull {
        services: Vec<String>,
        #[arg(long)]
        ignore_pull_failures: bool,
        #[arg(short, long)]
        quiet: bool,
    },

    /// Build or rebuild service images.
    Build {
        services: Vec<String>,
        #[arg(long)]
        no_cache: bool,
        #[arg(long)]
        pull: bool,
        #[arg(short, long)]
        quiet: bool,
    },

    /// Create service containers without starting them.
    Create {
        services: Vec<String>,
        #[arg(long)]
        build: bool,
        #[arg(long, conflicts_with = "no_recreate")]
        force_recreate: bool,
        #[arg(long, conflicts_with = "force_recreate")]
        no_recreate: bool,
        #[arg(long, value_enum, default_value_t = PullPolicy::Missing)]
        pull: PullPolicy,
    },

    /// Create and start services.
    Up {
        services: Vec<String>,
        #[arg(short, long)]
        detach: bool,
        #[arg(long)]
        no_start: bool,
        #[arg(long, conflicts_with = "no_build")]
        build: bool,
        #[arg(long, conflicts_with = "build")]
        no_build: bool,
        #[arg(long, conflicts_with = "no_recreate")]
        force_recreate: bool,
        #[arg(long, conflicts_with = "force_recreate")]
        no_recreate: bool,
        #[arg(long, value_enum, default_value_t = PullPolicy::Missing)]
        pull: PullPolicy,
        #[arg(long)]
        remove_orphans: bool,
    },

    /// Stop and remove project containers and networks.
    Down {
        #[arg(short = 'v', long)]
        volumes: bool,
        #[arg(long, default_value_t = 10)]
        timeout: u64,
        #[arg(long)]
        remove_orphans: bool,
    },

    /// Start existing service containers.
    Start { services: Vec<String> },

    /// Stop running service containers.
    Stop {
        services: Vec<String>,
        #[arg(short = 't', long, default_value_t = 10)]
        timeout: u64,
    },

    /// Restart service containers.
    Restart {
        services: Vec<String>,
        #[arg(short = 't', long, default_value_t = 10)]
        timeout: u64,
    },

    /// List project containers.
    #[command(alias = "ls")]
    Ps {
        services: Vec<String>,
        #[arg(short, long)]
        all: bool,
        #[arg(short, long)]
        quiet: bool,
        #[arg(long, value_enum, default_value_t = PsFormat::Table)]
        format: PsFormat,
    },

    /// Display service logs.
    Logs {
        services: Vec<String>,
        #[arg(short, long)]
        follow: bool,
        #[arg(short = 'n', long)]
        tail: Option<u64>,
        #[arg(short = 't', long)]
        timestamps: bool,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
    },

    /// Display a resource usage snapshot for service containers.
    Stats {
        services: Vec<String>,
        #[arg(short, long)]
        all: bool,
        #[arg(long)]
        no_trunc: bool,
        #[arg(long, value_enum, default_value_t = PsFormat::Table)]
        format: PsFormat,
    },

    /// Execute a command in a running service container.
    Exec {
        #[arg(short, long)]
        detach: bool,
        #[arg(short, long)]
        interactive: bool,
        #[arg(short, long)]
        tty: bool,
        #[arg(short = 'e', long = "env", action = ArgAction::Append)]
        environment: Vec<String>,
        #[arg(short, long)]
        user: Option<String>,
        #[arg(short, long)]
        workdir: Option<String>,
        service: String,
        #[arg(required = true, trailing_var_arg = true)]
        command: Vec<String>,
    },

    /// Run a one-off command for a service.
    Run {
        #[arg(short, long)]
        detach: bool,
        #[arg(long)]
        rm: bool,
        #[arg(long)]
        no_deps: bool,
        #[arg(short = 'e', long = "env", action = ArgAction::Append)]
        environment: Vec<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(short, long)]
        service_ports: bool,
        service: String,
        #[arg(trailing_var_arg = true)]
        command: Vec<String>,
    },

    /// Send SIGKILL or another signal to service containers.
    Kill {
        services: Vec<String>,
        #[arg(short, long, default_value = "SIGKILL")]
        signal: String,
    },

    /// Remove stopped service containers.
    #[command(alias = "remove")]
    Rm {
        services: Vec<String>,
        #[arg(short, long)]
        force: bool,
        #[arg(short, long)]
        stop: bool,
        #[arg(short = 'v', long)]
        volumes: bool,
    },

    /// Show wslc-compose and WSLC versions.
    Version {
        #[arg(long)]
        short: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum PullPolicy {
    Always,
    #[default]
    Missing,
    Never,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Yaml,
    Json,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum PsFormat {
    #[default]
    Table,
    Json,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn clap_command_tree_has_no_argument_conflicts() {
        Cli::command().debug_assert();
    }

    #[test]
    fn compose_file_and_logs_follow_can_both_use_short_f() {
        let cli = Cli::try_parse_from(["wslc-compose", "-f", "compose.yaml", "logs", "-f", "web"])
            .unwrap();
        assert_eq!(cli.files, [PathBuf::from("compose.yaml")]);
        assert!(matches!(cli.command, Command::Logs { follow: true, .. }));
    }
}
