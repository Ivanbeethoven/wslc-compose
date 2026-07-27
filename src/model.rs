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
    pub dependency_conditions: IndexMap<String, DependencyCondition>,
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
    pub healthcheck: Option<Healthcheck>,
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
pub struct Healthcheck {
    pub command: Option<String>,
    pub interval: Option<String>,
    pub timeout: Option<String>,
    pub start_period: Option<String>,
    pub retries: Option<u32>,
    pub disabled: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DependencyCondition {
    #[default]
    Started,
    Healthy,
    CompletedSuccessfully,
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
    pub ulimits: IndexMap<String, Value>,
    #[serde(default)]
    pub healthcheck: Option<RawHealthcheck>,
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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RawHealthcheck {
    #[serde(default)]
    pub test: Value,
    #[serde(default)]
    pub interval: Option<String>,
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(default)]
    pub start_period: Option<String>,
    #[serde(default)]
    pub retries: Option<u32>,
    #[serde(default)]
    pub disable: bool,
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

pub struct NormalizeOptions<'a> {
    pub project_name: &'a str,
    pub working_dir: &'a Path,
    pub source_files: Vec<PathBuf>,
    pub host_env: &'a Environment,
}

impl Project {
    pub fn normalize(raw: RawCompose, options: NormalizeOptions<'_>) -> Result<Self> {
        if raw.services.is_empty() {
            return Err(Error::InvalidConfig(
                "Compose project must define at least one service".to_owned(),
            ));
        }

        let mut networks = normalize_resources(
            &raw.networks,
            options.project_name,
            "network",
            options.host_env,
        )?;
        networks.entry("default".to_owned()).or_insert(Resource {
            key: "default".to_owned(),
            name: format!("{}_default", options.project_name),
            external: false,
            driver: Some("bridge".to_owned()),
            labels: IndexMap::new(),
        });

        let volumes = normalize_resources(
            &raw.volumes,
            options.project_name,
            "volume",
            options.host_env,
        )?;

        let mut services = IndexMap::new();
        for (name, raw_service) in raw.services {
            let service = normalize_service(
                &name,
                raw_service,
                options.project_name,
                options.working_dir,
                &networks,
                &volumes,
                options.host_env,
            )?;
            services.insert(name, service);
        }

        for service in services.values() {
            for dependency in &service.depends_on {
                if !services.contains_key(dependency) {
                    return Err(Error::InvalidConfig(format!(
                        "service {} depends on unknown service {dependency}",
                        service.name
                    )));
                }
            }
        }

        Ok(Self {
            name: options.project_name.to_owned(),
            working_dir: options.working_dir.to_path_buf(),
            source_files: options.source_files,
            services,
            networks,
            volumes,
        })
    }
}

fn normalize_ulimits(raw: &IndexMap<String, Value>) -> Result<IndexMap<String, String>> {
    let mut result = IndexMap::new();
    for (name, value) in raw.iter() {
        let formatted = match value {
            Value::Number(n) => format!("{name}={n}"),
            Value::Mapping(m) => {
                let soft = m
                    .get(Value::String("soft".to_owned()))
                    .and_then(Value::as_i64)
                    .ok_or_else(|| {
                        Error::InvalidConfig(format!("ulimit {name}: missing or invalid 'soft'"))
                    })?;
                let hard = m
                    .get(Value::String("hard".to_owned()))
                    .and_then(Value::as_i64)
                    .unwrap_or(soft);
                format!("{name}={soft}:{hard}")
            }
            _ => {
                return Err(Error::InvalidConfig(format!(
                    "ulimit {name}: must be a number or soft/hard map"
                )))
            }
        };
        result.insert(name.clone(), formatted);
    }
    Ok(result)
}

fn normalize_healthcheck(raw: &RawHealthcheck) -> Result<Healthcheck> {
    let mut disabled = raw.disable;
    let command = match &raw.test {
        Value::Null => None,
        Value::String(command) => Some(command.clone()),
        Value::Sequence(values) => {
            let values = values
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        Error::InvalidConfig(
                            "healthcheck.test entries must all be strings".to_owned(),
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let Some((mode, arguments)) = values.split_first() else {
                return Err(Error::InvalidConfig(
                    "healthcheck.test must not be empty".to_owned(),
                ));
            };
            match mode.as_str() {
                "NONE" => {
                    disabled = true;
                    None
                }
                "CMD-SHELL" => Some(arguments.join(" ")),
                "CMD" => Some(
                    shlex::try_join(arguments.iter().map(String::as_str)).map_err(|error| {
                        Error::InvalidConfig(format!(
                            "could not quote healthcheck command arguments: {error}"
                        ))
                    })?,
                ),
                _ => {
                    return Err(Error::InvalidConfig(format!(
                        "healthcheck.test mode must be CMD, CMD-SHELL, or NONE, got {mode}"
                    )))
                }
            }
        }
        _ => {
            return Err(Error::InvalidConfig(
                "healthcheck.test must be a string or list".to_owned(),
            ))
        }
    };
    if raw.retries.is_some_and(|retries| retries > i32::MAX as u32) {
        return Err(Error::InvalidConfig(
            "healthcheck.retries exceeds the WSLC integer range".to_owned(),
        ));
    }
    Ok(Healthcheck {
        command,
        interval: raw.interval.clone(),
        timeout: raw.timeout.clone(),
        start_period: raw.start_period.clone(),
        retries: raw.retries,
        disabled,
    })
}

fn normalize_service(
    name: &str,
    raw: RawService,
    project_name: &str,
    working_dir: &Path,
    networks: &IndexMap<String, Resource>,
    volumes: &IndexMap<String, Resource>,
    host_env: &Environment,
) -> Result<Service> {
    if raw
        .working_dir
        .as_deref()
        .is_some_and(|path| !path.starts_with('/'))
    {
        return Err(Error::InvalidConfig(format!(
            "service {name} working_dir must be an absolute Linux path"
        )));
    }
    let generated_tag = raw.image.is_none() && raw.build.is_some();
    let image = raw.image.clone().or_else(|| {
        raw.build
            .as_ref()
            .map(|_| format!("{project_name}-{name}:latest"))
    });
    let build = raw
        .build
        .as_ref()
        .map(|value| {
            normalize_build(
                value,
                project_name,
                name,
                working_dir,
                image.as_deref(),
                generated_tag,
                host_env,
            )
        })
        .transpose()?;
    let mut environment = IndexMap::new();
    for env_file in env_file_paths(&raw.env_file)? {
        let path = resolve_path(working_dir, &env_file.path);
        if !path.exists() {
            if env_file.required {
                return Err(Error::InvalidConfig(format!(
                    "service {name} env_file does not exist: {}",
                    path.display()
                )));
            }
            continue;
        }
        let values = dotenvy::from_path_iter(&path).map_err(|source| Error::ReadFile {
            path: path.clone(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
        })?;
        for value in values {
            let (key, value) = value.map_err(|source| Error::ReadFile {
                path: path.clone(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
            })?;
            environment.insert(key, value);
        }
    }
    environment.extend(parse_key_values(&raw.environment, host_env)?);

    let labels = parse_key_values(&raw.labels, host_env)?;
    let dependency_conditions = normalize_dependencies(&raw.depends_on)?;
    let depends_on = dependency_conditions.keys().cloned().collect();
    let service_networks = normalize_service_networks(&raw.networks, networks)?;
    let mounts = raw
        .volumes
        .iter()
        .map(|value| normalize_mount(value, project_name, working_dir, volumes))
        .collect::<Result<Vec<_>>>()?;
    let ports = raw
        .ports
        .iter()
        .filter_map(normalize_port)
        .collect::<Result<Vec<_>>>()?;

    let mut unsupported: Vec<_> = raw
        .extra
        .iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(name, _)| name.clone())
        .collect();
    if let Some(healthcheck) = &raw.healthcheck {
        unsupported.extend(
            healthcheck
                .extra
                .iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(name, _)| format!("healthcheck.{name}")),
        );
    }

    Ok(Service {
        name: name.to_owned(),
        image,
        build,
        container_name: raw
            .container_name
            .unwrap_or_else(|| format!("{project_name}-{name}-1")),
        command: raw
            .command
            .map(StringOrList::into_vec)
            .transpose()?
            .unwrap_or_default(),
        entrypoint: raw
            .entrypoint
            .map(StringOrList::into_vec)
            .transpose()?
            .unwrap_or_default(),
        environment,
        ports,
        mounts,
        depends_on,
        dependency_conditions,
        profiles: raw.profiles,
        labels,
        networks: service_networks,
        hostname: raw.hostname,
        domain_name: raw.domain_name,
        privileged: raw.privileged,
        working_dir: raw.working_dir,
        user: raw.user,
        tty: raw.tty,
        stdin_open: raw.stdin_open,
        gpus: raw.gpus.as_ref().is_some_and(|value| !value.is_null()),
        memory: raw.mem_limit.as_ref().and_then(scalar_string),
        cpus: raw.cpus.as_ref().and_then(scalar_string),
        ulimits: normalize_ulimits(&raw.ulimits)?,
        healthcheck: raw
            .healthcheck
            .as_ref()
            .map(normalize_healthcheck)
            .transpose()?,
        stop_signal: raw.stop_signal.unwrap_or_else(|| "SIGTERM".to_owned()),
        stop_grace_period: raw
            .stop_grace_period
            .as_deref()
            .map(parse_duration)
            .transpose()?
            .unwrap_or(Duration::from_secs(10)),
        restart: raw.restart,
        unsupported,
    })
}

fn normalize_build(
    value: &Value,
    project_name: &str,
    service_name: &str,
    working_dir: &Path,
    image: Option<&str>,
    generated_tag: bool,
    host_env: &Environment,
) -> Result<BuildConfig> {
    let (context, dockerfile, args, target, labels) = match value {
        Value::String(context) => (
            PathBuf::from(context),
            None,
            IndexMap::new(),
            None,
            IndexMap::new(),
        ),
        Value::Mapping(mapping) => {
            let context = mapping_get(mapping, "context")
                .and_then(Value::as_str)
                .unwrap_or(".");
            let dockerfile = mapping_get(mapping, "dockerfile")
                .and_then(Value::as_str)
                .map(PathBuf::from);
            let args = mapping_get(mapping, "args")
                .map(|value| parse_key_values(value, host_env))
                .transpose()?
                .unwrap_or_default();
            let target = mapping_get(mapping, "target")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let labels = mapping_get(mapping, "labels")
                .map(|value| parse_key_values(value, host_env))
                .transpose()?
                .unwrap_or_default();
            (PathBuf::from(context), dockerfile, args, target, labels)
        }
        _ => {
            return Err(Error::InvalidConfig(format!(
                "service {service_name} build must be a path or map"
            )))
        }
    };
    let context = resolve_path(working_dir, &context);
    let dockerfile = dockerfile.map(|path| resolve_path(&context, &path));
    Ok(BuildConfig {
        context,
        dockerfile,
        args,
        target,
        labels,
        tag: image
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{project_name}-{service_name}:latest")),
        generated_tag,
    })
}

#[derive(Debug)]
struct EnvFile {
    path: PathBuf,
    required: bool,
}

fn env_file_paths(value: &Value) -> Result<Vec<EnvFile>> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::String(path) => Ok(vec![EnvFile {
            path: PathBuf::from(path),
            required: true,
        }]),
        Value::Sequence(values) => values.iter().map(parse_env_file).collect(),
        Value::Mapping(_) => Ok(vec![parse_env_file(value)?]),
        _ => Err(Error::InvalidConfig(
            "env_file must be a path or list".to_owned(),
        )),
    }
}

fn parse_env_file(value: &Value) -> Result<EnvFile> {
    match value {
        Value::String(path) => Ok(EnvFile {
            path: PathBuf::from(path),
            required: true,
        }),
        Value::Mapping(mapping) => {
            let path = mapping_get(mapping, "path")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::InvalidConfig("env_file.path is required".to_owned()))?;
            let required = mapping_get(mapping, "required")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            Ok(EnvFile {
                path: PathBuf::from(path),
                required,
            })
        }
        _ => Err(Error::InvalidConfig("invalid env_file entry".to_owned())),
    }
}

fn parse_key_values(value: &Value, host_env: &Environment) -> Result<IndexMap<String, String>> {
    let mut result = IndexMap::new();
    match value {
        Value::Null => {}
        Value::Mapping(mapping) => {
            for (key, value) in mapping {
                let key = key
                    .as_str()
                    .ok_or_else(|| Error::InvalidConfig("map key must be a string".to_owned()))?;
                let value = if value.is_null() {
                    host_env.get(key).cloned().unwrap_or_default()
                } else {
                    scalar_string(value).ok_or_else(|| {
                        Error::InvalidConfig(format!("value for {key} must be scalar"))
                    })?
                };
                result.insert(key.to_owned(), value);
            }
        }
        Value::Sequence(values) => {
            for value in values {
                let entry = value.as_str().ok_or_else(|| {
                    Error::InvalidConfig("list entry must be a string".to_owned())
                })?;
                let (key, value) = entry
                    .split_once('=')
                    .map(|(key, value)| (key, value.to_owned()))
                    .unwrap_or_else(|| (entry, host_env.get(entry).cloned().unwrap_or_default()));
                result.insert(key.to_owned(), value);
            }
        }
        _ => return Err(Error::InvalidConfig("expected a map or list".to_owned())),
    }
    Ok(result)
}

fn normalize_dependencies(value: &Value) -> Result<IndexMap<String, DependencyCondition>> {
    match value {
        Value::Null => Ok(IndexMap::new()),
        Value::Sequence(values) => {
            let mut dependencies = IndexMap::new();
            for value in values {
                let name = value.as_str().ok_or_else(|| {
                    Error::InvalidConfig("dependency must be a string".to_owned())
                })?;
                dependencies.insert(name.to_owned(), DependencyCondition::Started);
            }
            Ok(dependencies)
        }
        Value::Mapping(mapping) => {
            let mut dependencies = IndexMap::new();
            for (key, options) in mapping {
                let name = key.as_str().ok_or_else(|| {
                    Error::InvalidConfig("dependency key must be a string".to_owned())
                })?;
                let condition = match options {
                    Value::Null => DependencyCondition::Started,
                    Value::Mapping(options) => match mapping_get(options, "condition")
                        .and_then(Value::as_str)
                        .unwrap_or("service_started")
                    {
                        "service_started" => DependencyCondition::Started,
                        "service_healthy" => DependencyCondition::Healthy,
                        "service_completed_successfully" => {
                            DependencyCondition::CompletedSuccessfully
                        }
                        condition => {
                            return Err(Error::InvalidConfig(format!(
                                "unsupported depends_on condition for {name}: {condition}"
                            )))
                        }
                    },
                    _ => {
                        return Err(Error::InvalidConfig(format!(
                            "depends_on options for {name} must be a map"
                        )))
                    }
                };
                dependencies.insert(name.to_owned(), condition);
            }
            Ok(dependencies)
        }
        _ => Err(Error::InvalidConfig(
            "depends_on must be a list or map".to_owned(),
        )),
    }
}

fn normalize_service_networks(
    value: &Value,
    resources: &IndexMap<String, Resource>,
) -> Result<Vec<ServiceNetwork>> {
    let entries: Vec<(String, Vec<String>)> = match value {
        Value::Null => vec![("default".to_owned(), Vec::new())],
        Value::Sequence(values) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(|name| (name.to_owned(), Vec::new()))
                    .ok_or_else(|| Error::InvalidConfig("network must be a string".to_owned()))
            })
            .collect::<Result<_>>()?,
        Value::Mapping(mapping) => mapping
            .iter()
            .map(|(key, value)| {
                let key = key.as_str().ok_or_else(|| {
                    Error::InvalidConfig("network key must be a string".to_owned())
                })?;
                let aliases = value
                    .as_mapping()
                    .and_then(|mapping| mapping_get(mapping, "aliases"))
                    .and_then(Value::as_sequence)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                Ok((key.to_owned(), aliases))
            })
            .collect::<Result<_>>()?,
        _ => {
            return Err(Error::InvalidConfig(
                "networks must be a list or map".to_owned(),
            ))
        }
    };

    entries
        .into_iter()
        .map(|(key, aliases)| {
            let resource = resources.get(&key).ok_or_else(|| {
                Error::InvalidConfig(format!("service references undeclared network {key}"))
            })?;
            Ok(ServiceNetwork {
                name: resource.name.clone(),
                aliases,
            })
        })
        .collect()
}

fn normalize_resources(
    values: &IndexMap<String, Value>,
    project_name: &str,
    kind: &str,
    host_env: &Environment,
) -> Result<IndexMap<String, Resource>> {
    values
        .iter()
        .map(|(key, value)| {
            let mapping = value.as_mapping();
            let external = mapping
                .and_then(|mapping| mapping_get(mapping, "external"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let explicit_name = mapping
                .and_then(|mapping| mapping_get(mapping, "name"))
                .and_then(Value::as_str);
            let name = explicit_name
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{project_name}_{key}"));
            let driver = mapping
                .and_then(|mapping| mapping_get(mapping, "driver"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let labels = mapping
                .and_then(|mapping| mapping_get(mapping, "labels"))
                .map(|value| parse_key_values(value, host_env))
                .transpose()?
                .unwrap_or_default();
            if external && explicit_name.is_none() && key.is_empty() {
                return Err(Error::InvalidConfig(format!(
                    "external {kind} must have a name"
                )));
            }
            Ok((
                key.clone(),
                Resource {
                    key: key.clone(),
                    name,
                    external,
                    driver,
                    labels,
                },
            ))
        })
        .collect()
}

fn normalize_port(value: &Value) -> Option<Result<String>> {
    match value {
        Value::String(value) => Some(Ok(value.clone())),
        Value::Number(value) => Some(Ok(value.to_string())),
        Value::Mapping(mapping) => {
            let target = mapping_get(mapping, "target").and_then(scalar_string);
            let published = mapping_get(mapping, "published").and_then(scalar_string);
            let host_ip = mapping_get(mapping, "host_ip").and_then(Value::as_str);
            let protocol = mapping_get(mapping, "protocol").and_then(Value::as_str);
            Some(match (target, published) {
                (Some(target), Some(published)) => {
                    let host = host_ip.map(|host| format!("{host}:")).unwrap_or_default();
                    let protocol = protocol
                        .filter(|protocol| !protocol.eq_ignore_ascii_case("tcp"))
                        .map(|protocol| format!("/{protocol}"))
                        .unwrap_or_default();
                    Ok(format!("{host}{published}:{target}{protocol}"))
                }
                (Some(_), None) => return None,
                _ => Err(Error::InvalidConfig("port target is required".to_owned())),
            })
        }
        _ => Some(Err(Error::InvalidConfig("invalid port entry".to_owned()))),
    }
}

fn normalize_mount(
    value: &Value,
    project_name: &str,
    working_dir: &Path,
    volumes: &IndexMap<String, Resource>,
) -> Result<Mount> {
    match value {
        Value::String(value) => normalize_short_mount(value, project_name, working_dir, volumes),
        Value::Mapping(mapping) => {
            let kind = mapping_get(mapping, "type")
                .and_then(Value::as_str)
                .unwrap_or("volume");
            let source = mapping_get(mapping, "source").and_then(Value::as_str);
            let target = mapping_get(mapping, "target")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::InvalidConfig("volume target is required".to_owned()))?;
            validate_container_path(target)?;
            let read_only = mapping_get(mapping, "read_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            match kind {
                "bind" => {
                    let source = source.ok_or_else(|| {
                        Error::InvalidConfig("bind mount source is required".to_owned())
                    })?;
                    Ok(Mount::Bind {
                        source: resolve_path(working_dir, Path::new(source)),
                        target: target.to_owned(),
                        read_only,
                    })
                }
                "volume" => match source {
                    Some(source) => Ok(Mount::Volume {
                        source: volume_name(source, project_name, volumes),
                        target: target.to_owned(),
                        read_only,
                    }),
                    None => Ok(Mount::Anonymous {
                        target: target.to_owned(),
                        read_only,
                    }),
                },
                "tmpfs" => Ok(Mount::Tmpfs {
                    target: target.to_owned(),
                }),
                other => Err(Error::InvalidConfig(format!(
                    "unsupported volume type {other}"
                ))),
            }
        }
        _ => Err(Error::InvalidConfig("invalid volume entry".to_owned())),
    }
}

fn normalize_short_mount(
    value: &str,
    project_name: &str,
    working_dir: &Path,
    volumes: &IndexMap<String, Resource>,
) -> Result<Mount> {
    if value.starts_with('/') {
        return Ok(Mount::Anonymous {
            target: value.to_owned(),
            read_only: false,
        });
    }

    let (without_mode, mode) = value
        .rsplit_once(':')
        .filter(|(_, mode)| is_mount_mode(mode))
        .map(|(value, mode)| (value, Some(mode)))
        .unwrap_or((value, None));
    let read_only = mode.is_some_and(|mode| mode.split(',').any(|part| part == "ro"));

    let separator = without_mode.rfind(":/").ok_or_else(|| {
        Error::InvalidConfig(format!(
            "volume must include an absolute container path: {value}"
        ))
    })?;
    let source = &without_mode[..separator];
    let target = &without_mode[separator + 1..];
    validate_container_path(target)?;
    if is_host_path(source) {
        Ok(Mount::Bind {
            source: resolve_path(working_dir, Path::new(source)),
            target: target.to_owned(),
            read_only,
        })
    } else {
        Ok(Mount::Volume {
            source: volume_name(source, project_name, volumes),
            target: target.to_owned(),
            read_only,
        })
    }
}

fn validate_container_path(path: &str) -> Result<()> {
    if path.starts_with('/') {
        Ok(())
    } else {
        Err(Error::InvalidConfig(format!(
            "container path must be an absolute Linux path: {path}"
        )))
    }
}

fn is_mount_mode(value: &str) -> bool {
    value.split(',').all(|part| {
        matches!(
            part,
            "ro" | "rw" | "z" | "Z" | "cached" | "delegated" | "consistent" | "nocopy"
        )
    })
}

fn volume_name(source: &str, project_name: &str, volumes: &IndexMap<String, Resource>) -> String {
    volumes
        .get(source)
        .map(|resource| resource.name.clone())
        .unwrap_or_else(|| format!("{project_name}_{source}"))
}

fn is_host_path(value: &str) -> bool {
    value.starts_with('.')
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.as_bytes().get(1) == Some(&b':')
}

fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn parse_duration(value: &str) -> Result<Duration> {
    let mut total_ms = 0u64;
    let mut start = 0usize;
    let bytes = value.as_bytes();
    while start < bytes.len() {
        let mut end = start;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if end == start {
            return Err(Error::InvalidConfig(format!("invalid duration: {value}")));
        }
        let amount: u64 = value[start..end]
            .parse()
            .map_err(|_| Error::InvalidConfig(format!("invalid duration: {value}")))?;
        let (factor, next) = if value[end..].starts_with("ms") {
            (1, end + 2)
        } else if value[end..].starts_with('s') {
            (1_000, end + 1)
        } else if value[end..].starts_with('m') {
            (60_000, end + 1)
        } else if value[end..].starts_with('h') {
            (3_600_000, end + 1)
        } else {
            return Err(Error::InvalidConfig(format!(
                "invalid duration unit: {value}"
            )));
        };
        total_ms = total_ms
            .checked_add(amount.saturating_mul(factor))
            .ok_or_else(|| Error::InvalidConfig(format!("duration is too large: {value}")))?;
        start = next;
    }
    Ok(Duration::from_millis(total_ms))
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn mapping_get<'a>(mapping: &'a serde_yaml::Mapping, key: &str) -> Option<&'a Value> {
    mapping.get(Value::String(key.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_windows_bind_and_named_volume_mounts() {
        let volumes = IndexMap::from([(
            "data".to_owned(),
            Resource {
                key: "data".to_owned(),
                name: "demo_data".to_owned(),
                external: false,
                driver: None,
                labels: IndexMap::new(),
            },
        )]);
        assert_eq!(
            normalize_short_mount(
                r"C:\work\config:/etc/app:ro",
                "demo",
                Path::new(r"C:\project"),
                &volumes,
            )
            .unwrap(),
            Mount::Bind {
                source: PathBuf::from(r"C:\work\config"),
                target: "/etc/app".to_owned(),
                read_only: true,
            }
        );
        assert_eq!(
            normalize_short_mount("data:/var/lib/app", "demo", Path::new("."), &volumes).unwrap(),
            Mount::Volume {
                source: "demo_data".to_owned(),
                target: "/var/lib/app".to_owned(),
                read_only: false,
            }
        );
        assert_eq!(
            normalize_short_mount("data:/var/lib/app:rw", "demo", Path::new("."), &volumes)
                .unwrap(),
            Mount::Volume {
                source: "demo_data".to_owned(),
                target: "/var/lib/app".to_owned(),
                read_only: false,
            }
        );
    }

    #[test]
    fn parses_compose_durations() {
        assert_eq!(parse_duration("1m30s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("250ms").unwrap(), Duration::from_millis(250));
    }

    #[test]
    fn normalizes_compose_healthcheck_for_wslc_cli() {
        let raw: RawHealthcheck = serde_yaml::from_str(
            r#"
test: ["CMD", "redis-cli", "-h", "cache service", "ping"]
interval: 5s
timeout: 2s
start_period: 10s
retries: 4
"#,
        )
        .unwrap();
        let healthcheck = normalize_healthcheck(&raw).unwrap();
        assert_eq!(
            healthcheck.command.as_deref(),
            Some("redis-cli -h 'cache service' ping")
        );
        assert_eq!(healthcheck.interval.as_deref(), Some("5s"));
        assert_eq!(healthcheck.timeout.as_deref(), Some("2s"));
        assert_eq!(healthcheck.start_period.as_deref(), Some("10s"));
        assert_eq!(healthcheck.retries, Some(4));
        assert!(!healthcheck.disabled);
    }

    #[test]
    fn healthcheck_none_disables_image_healthcheck() {
        let raw: RawHealthcheck = serde_yaml::from_str("test: [\"NONE\"]").unwrap();
        let healthcheck = normalize_healthcheck(&raw).unwrap();
        assert!(healthcheck.disabled);
        assert!(healthcheck.command.is_none());
    }

    #[test]
    fn normalizes_long_dependency_conditions() {
        let value: Value = serde_yaml::from_str(
            r#"
database:
  condition: service_healthy
migrate:
  condition: service_completed_successfully
cache:
  condition: service_started
"#,
        )
        .unwrap();
        let dependencies = normalize_dependencies(&value).unwrap();
        assert_eq!(dependencies["database"], DependencyCondition::Healthy);
        assert_eq!(
            dependencies["migrate"],
            DependencyCondition::CompletedSuccessfully
        );
        assert_eq!(dependencies["cache"], DependencyCondition::Started);
    }
}
