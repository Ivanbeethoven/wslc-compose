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

#[derive(Clone, Debug)]
pub struct BuildConfig {
    pub context: PathBuf,
    pub dockerfile: Option<PathBuf>,
    pub args: IndexMap<String, String>,
    pub target: Option<String>,
    pub labels: IndexMap<String, String>,
    pub tag: String,
    pub generated_tag: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Mount {
    Bind {
        source: PathBuf,
        target: String,
        read_only: bool,
    },
    Volume {
        source: String,
        target: String,
        read_only: bool,
    },
    Anonymous {
        target: String,
        read_only: bool,
    },
    Tmpfs {
        target: String,
    },
}

impl Mount {
    pub fn as_cli_value(&self) -> String {
        match self {
            Self::Bind {
                source,
                target,
                read_only,
            } => mount_value(&source.display().to_string(), target, *read_only),
            Self::Volume {
                source,
                target,
                read_only,
            } => mount_value(source, target, *read_only),
            Self::Anonymous { target, read_only } => {
                if *read_only {
                    format!("{target}:ro")
                } else {
                    target.clone()
                }
            }
            Self::Tmpfs { target } => target.clone(),
        }
    }
}

fn mount_value(source: &str, target: &str, read_only: bool) -> String {
    if read_only {
        format!("{source}:{target}:ro")
    } else {
        format!("{source}:{target}")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceNetwork {
    pub name: String,
    pub aliases: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Resource {
    pub key: String,
    pub name: String,
    pub external: bool,
    pub driver: Option<String>,
    pub labels: IndexMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RawCompose {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub services: IndexMap<String, RawService>,
    #[serde(default)]
    pub networks: IndexMap<String, Value>,
    #[serde(default)]
    pub volumes: IndexMap<String, Value>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RawService {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub build: Option<Value>,
    #[serde(default)]
    pub command: Option<StringOrList>,
    #[serde(default)]
    pub entrypoint: Option<StringOrList>,
    #[serde(default)]
    pub container_name: Option<String>,
    #[serde(default)]
    pub environment: Value,
    #[serde(default)]
    pub env_file: Value,
    #[serde(default)]
    pub ports: Vec<Value>,
    #[serde(default)]
    pub volumes: Vec<Value>,
    #[serde(default)]
    pub depends_on: Value,
    #[serde(default)]
    pub profiles: IndexSet<String>,
    #[serde(default)]
    pub labels: Value,
    #[serde(default)]
    pub networks: Value,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default, rename = "domainname")]
    pub domain_name: Option<String>,
    #[serde(default)]
    pub privileged: bool,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub tty: bool,
    #[serde(default)]
    pub stdin_open: bool,
    #[serde(default)]
    pub gpus: Option<Value>,
    #[serde(default)]
    pub mem_limit: Option<Value>,
    #[serde(default)]
    pub cpus: Option<Value>,
    #[serde(default)]
    pub stop_signal: Option<String>,
    #[serde(default)]
    pub stop_grace_period: Option<String>,
    #[serde(default)]
    pub restart: Option<String>,
    #[serde(default)]
    pub pull_policy: Option<String>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StringOrList {
    String(String),
    List(Vec<String>),
}

impl StringOrList {
    fn into_vec(self) -> Result<Vec<String>> {
        match self {
            Self::List(values) => Ok(values),
            Self::String(value) => shlex::split(&value).ok_or_else(|| {
                Error::InvalidConfig(format!("could not parse command line: {value}"))
            }),
        }
    }
}
