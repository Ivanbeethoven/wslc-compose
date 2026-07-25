use std::path::{Path, PathBuf};
use std::time::Duration;

use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::env::Environment;
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct Project {
    pub name: String,
    pub working_dir: PathBuf,
    pub source_files: Vec<PathBuf>,
    pub services: IndexMap<String, Service>,
    pub networks: IndexMap<String, Resource>,
    pub volumes: IndexMap<String, Resource>,
}

#[derive(Clone, Debug)]
pub struct Service {
    pub name: String,
    pub image: Option<String>,
    pub build: Option<BuildConfig>,
    pub container_name: String,
    pub command: Vec<String>,
    pub entrypoint: Vec<String>,
    pub environment: IndexMap<String, String>,
    pub ports: Vec<String>,
    pub mounts: Vec<Mount>,
    pub depends_on: Vec<String>,
    pub profiles: IndexSet<String>,
    pub labels: IndexMap<String, String>,
    pub networks: Vec<ServiceNetwork>,
    pub hostname: Option<String>,
    pub domain_name: Option<String>,
    pub privileged: bool,
    pub working_dir: Option<String>,
    pub user: Option<String>,
    pub tty: bool,
    pub stdin_open: bool,
    pub gpus: bool,
    pub memory: Option<String>,
    pub cpus: Option<String>,
    pub ulimits: IndexMap<String, String>,
    pub stop_signal: String,
    pub stop_grace_period: Duration,
    pub restart: Option<String>,
    pub unsupported: Vec<String>,
}
