use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{BuildConfig, DependencyCondition, Mount, Project, Resource, Service};
use crate::sdk_daemon::Client as SdkClient;
use crate::{Error, Result};

const DEPENDENCY_WAIT_TIMEOUT_ENV: &str = "WSLC_COMPOSE_WAIT_TIMEOUT_SECS";
const DEFAULT_DEPENDENCY_WAIT_TIMEOUT_SECS: u64 = 120;
const CONFIG_HASH_LABEL: &str = "com.docker.compose.config-hash";

pub struct WslcBackend {
    program: OsString,
    working_dir: PathBuf,
    sdk: SdkClient,
}

pub struct Availability {
    pub cli_version: String,
    pub sdk_version: Option<wslc::Version>,
}

pub struct OneOffOptions<'a> {
    pub name: &'a str,
    pub command: &'a [String],
    pub environment: &'a [String],
    pub detach: bool,
    pub remove: bool,
    pub service_ports: bool,
}

pub struct ExecOptions<'a> {
    pub command: &'a [String],
    pub environment: &'a [String],
    pub detach: bool,
    pub interactive: bool,
    pub tty: bool,
    pub user: Option<&'a str>,
    pub workdir: Option<&'a str>,
}

pub struct LogOptions<'a> {
    pub follow: bool,
    pub tail: Option<u64>,
    pub timestamps: bool,
    pub since: Option<&'a str>,
    pub until: Option<&'a str>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ListedContainer {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Image")]
    image: String,
    #[serde(rename = "State")]
    state: i32,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct InspectedContainer {
    #[serde(rename = "State")]
    state: InspectedContainerState,
    #[serde(rename = "Labels", default)]
    labels: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct InspectedContainerState {
    #[serde(rename = "Status")]
    status: String,
    #[serde(rename = "Running")]
    running: bool,
    #[serde(rename = "ExitCode")]
    exit_code: i32,
    #[serde(rename = "Health")]
    health: Option<InspectedHealth>,
}

#[derive(Debug, Deserialize)]
struct InspectedHealth {
    #[serde(rename = "Status")]
    status: String,
}

impl WslcBackend {
    pub fn new(working_dir: impl Into<PathBuf>) -> Self {
        Self {
            program: OsString::from("wslc"),
            working_dir: working_dir.into(),
            sdk: SdkClient::new(),
        }
    }

    pub fn ensure_available(&self) -> Result<Availability> {
        let cli_version = self.capture(&["version".to_owned()])?.trim().to_owned();
        let sdk_version = match wslc::Service::ensure_available() {
            Ok(()) => Some(wslc::Service::version()?),
            Err(wslc::Error::SdkNotFound(_)) => None,
            Err(error) => return Err(error.into()),
        };
        Ok(Availability {
            cli_version,
            sdk_version,
        })
    }

    pub fn pull(&self, image: &str, quiet: bool) -> Result<()> {
        self.inherit_with_quiet(&["pull".to_owned(), resolve_image_reference(image)?], quiet)
    }

    pub fn image_exists(&self, image: &str) -> Result<bool> {
        let image = resolve_image_reference(image)?;
        let status = self.status_quiet(&["image".to_owned(), "inspect".to_owned(), image])?;
        Ok(status.success())
    }

    pub fn container_matches_configuration(&self, name: &str, service: &Service) -> Result<bool> {
        let expected = service_config_hash(service)?;
        let inspected = self.inspect_container(name)?;
        Ok(inspected.labels.get(CONFIG_HASH_LABEL) == Some(&expected))
    }

    pub fn build(
        &self,
        build: &BuildConfig,
        no_cache: bool,
        pull: bool,
        quiet: bool,
    ) -> Result<()> {
        let mut args = vec!["build".to_owned(), "--tag".to_owned(), build.tag.clone()];
        if no_cache {
            args.push("--no-cache".to_owned());
        }
        if pull {
            args.push("--pull".to_owned());
        }
        if let Some(dockerfile) = &build.dockerfile {
            args.extend(["--file".to_owned(), dockerfile.display().to_string()]);
        }
        if let Some(target) = &build.target {
            args.extend(["--target".to_owned(), target.clone()]);
        }
        for (key, value) in &build.args {
            args.extend(["--build-arg".to_owned(), format!("{key}={value}")]);
        }
        for (key, value) in &build.labels {
            args.extend(["--label".to_owned(), format!("{key}={value}")]);
        }
        args.push(build.context.display().to_string());
        self.inherit_with_quiet(&args, quiet)
    }

    pub fn ensure_project_resources(&self, project: &Project) -> Result<()> {
        if self.uses_sdk_project(project) {
            return Ok(());
        }
        for resource in project.networks.values() {
            self.ensure_resource("network", resource, &project.name)?;
        }
        for resource in project.volumes.values() {
            self.ensure_resource("volume", resource, &project.name)?;
        }
        Ok(())
    }

    pub fn remove_project_resources(&self, project: &Project, volumes: bool) -> Result<()> {
        if self.uses_sdk_project(project) {
            return Ok(());
        }
        for resource in project.networks.values().rev() {
            if !resource.external {
                self.remove_resource("network", &resource.name)?;
            }
        }
        if volumes {
            for resource in project.volumes.values().rev() {
                if !resource.external {
                    self.remove_resource("volume", &resource.name)?;
                }
            }
        }
        Ok(())
    }

    pub fn container_exists(&self, name: &str) -> Result<bool> {
        if self.sdk.existing(name)?.unwrap_or(false) {
            return Ok(true);
        }
        let status = self.status_quiet(&[
            "inspect".to_owned(),
            "--type".to_owned(),
            "container".to_owned(),
            name.to_owned(),
        ])?;
        Ok(status.success())
    }

    pub fn project_container_exists(&self, project: &Project, name: &str) -> Result<bool> {
        if self.uses_sdk_project(project) {
            return Ok(self.sdk.existing(name)?.unwrap_or(false));
        }
        self.container_exists(name)
    }

    pub fn container_running(&self, name: &str) -> Result<bool> {
        if self.sdk.running(name)?.unwrap_or(false) {
            return Ok(true);
        }
        let output = self.capture(&[
            "list".to_owned(),
            "--filter".to_owned(),
            format!("name={name}"),
            "--quiet".to_owned(),
        ])?;
        Ok(output.lines().any(|line| !line.trim().is_empty()))
    }

    pub fn project_container_running(&self, project: &Project, name: &str) -> Result<bool> {
        if self.uses_sdk_project(project) {
            return Ok(self.sdk.running(name)?.unwrap_or(false));
        }
        self.container_running(name)
    }

    pub fn create(&self, project: &Project, service: &Service) -> Result<()> {
        if self.uses_sdk_project(project) {
            return self.sdk.create(project, service);
        }
        let args = create_args(
            project,
            service,
            &service.container_name,
            false,
            true,
            &[],
            &[],
        )?;
        self.inherit(&args)
    }

    pub fn run_one_off(
        &self,
        project: &Project,
        service: &Service,
        options: OneOffOptions<'_>,
    ) -> Result<()> {
        if self.uses_sdk_project(project) {
            return Err(Error::Unsupported {
                service: service.name.clone(),
                feature: "run is not implemented for SDK-backed Compose projects".to_owned(),
            });
        }
        let args = create_args(
            project,
            service,
            options.name,
            options.remove,
            options.service_ports,
            options.command,
            options.environment,
        )?;
        let mut run_args = vec!["run".to_owned()];
        if options.detach {
            run_args.push("--detach".to_owned());
        }
        run_args.extend(args.into_iter().skip(1));
        self.inherit(&run_args)
    }

    pub fn start(&self, name: &str) -> Result<()> {
        if self.sdk.start(name)? {
            return Ok(());
        }
        self.inherit(&["start".to_owned(), name.to_owned()])
    }

    pub fn stop(&self, name: &str, signal: &str, timeout: u64) -> Result<()> {
        if self.sdk.stop(name, signal, timeout)? {
            return Ok(());
        }
        self.inherit(&[
            "stop".to_owned(),
            "--signal".to_owned(),
            signal.to_owned(),
            "--time".to_owned(),
            timeout.to_string(),
            name.to_owned(),
        ])
    }

    pub fn kill(&self, name: &str, signal: &str) -> Result<()> {
        if self.sdk.stop(name, signal, 0)? {
            return Ok(());
        }
        self.inherit(&[
            "kill".to_owned(),
            "--signal".to_owned(),
            signal.to_owned(),
            name.to_owned(),
        ])
    }

    pub fn remove(&self, name: &str, force: bool) -> Result<()> {
        if self.sdk.remove(name, force)? {
            return Ok(());
        }
        let mut args = vec!["remove".to_owned()];
        if force {
            args.push("--force".to_owned());
        }
        args.push(name.to_owned());
        self.inherit(&args)
    }

    pub fn ps(
        &self,
        project: &Project,
        all: bool,
        quiet: bool,
        json: bool,
        container_names: &[String],
    ) -> Result<()> {
        if self.uses_sdk_project(project) {
            let mut containers = self.sdk.list(&project.name)?.unwrap_or_default();
            if !all {
                containers.retain(|container| container.state == "Running");
            }
            if !container_names.is_empty() {
                containers.retain(|container| container_names.contains(&container.name));
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&containers)?);
            } else if quiet {
                for container in containers {
                    println!("{}", container.name);
                }
            } else {
                println!("NAME\tSTATE");
                for container in containers {
                    println!("{}\t{}", container.name, container.state);
                }
            }
            return Ok(());
        }
        if container_names.is_empty() {
            return self.inherit(&project_list_args(project, all, quiet, json));
        }

        let mut containers = self.list_project_containers(project, all)?;
        containers.retain(|container| container_names.contains(&container.name));
        if json {
            println!("{}", serde_json::to_string_pretty(&containers)?);
        } else if quiet {
            for container in containers {
                println!("{}", container.id);
            }
        } else {
            println!("CONTAINER ID\tNAME\tIMAGE\tSTATE");
            for container in containers {
                println!(
                    "{}\t{}\t{}\t{}",
                    truncate_id(&container.id),
                    container.name,
                    container.image,
                    container_state(container.state)
                );
            }
        }
        Ok(())
    }

    pub fn remove_orphans(&self, project: &Project) -> Result<Vec<String>> {
        let expected: BTreeSet<&str> = project
            .services
            .values()
            .map(|service| service.container_name.as_str())
            .collect();
        let existing = if self.uses_sdk_project(project) {
            self.sdk
                .list(&project.name)?
                .unwrap_or_default()
                .into_iter()
                .map(|container| container.name)
                .collect()
        } else {
            self.list_project_containers(project, true)?
                .into_iter()
                .map(|container| container.name)
                .collect::<Vec<_>>()
        };
        let orphans = orphan_names(existing, &expected);
        for name in &orphans {
            self.remove(name, true)?;
        }
        Ok(orphans)
    }

    pub fn wait_for_dependency(&self, name: &str, condition: DependencyCondition) -> Result<()> {
        if condition == DependencyCondition::Started {
            return Ok(());
        }
        if self.sdk.existing(name)?.unwrap_or(false) {
            return Err(Error::Unsupported {
                service: name.to_owned(),
                feature: "depends_on conditions are not exposed by the WSLC SDK backend".to_owned(),
            });
        }

        let timeout = dependency_wait_timeout()?;
        let started = Instant::now();
        loop {
            let state = self.inspect_container_state(name)?;
            match condition {
                DependencyCondition::Started => return Ok(()),
                DependencyCondition::Healthy => {
                    let Some(health) = state.health else {
                        return Err(Error::InvalidConfig(format!(
                            "dependency {name} uses service_healthy but has no healthcheck"
                        )));
                    };
                    if health.status.eq_ignore_ascii_case("healthy") {
                        return Ok(());
                    }
                    if !state.running {
                        return Err(Error::InvalidConfig(format!(
                            "dependency {name} stopped before becoming healthy"
                        )));
                    }
                }
                DependencyCondition::CompletedSuccessfully => {
                    if state.status.eq_ignore_ascii_case("exited") {
                        if state.exit_code == 0 {
                            return Ok(());
                        }
                        return Err(Error::InvalidConfig(format!(
                            "dependency {name} exited with status {}",
                            state.exit_code
                        )));
                    }
                    if !state.running && !state.status.eq_ignore_ascii_case("created") {
                        return Err(Error::InvalidConfig(format!(
                            "dependency {name} entered unexpected state {}",
                            state.status
                        )));
                    }
                }
            }
            if timeout.is_some_and(|timeout| started.elapsed() >= timeout) {
                return Err(Error::InvalidConfig(format!(
                    "timed out waiting for dependency {name} to satisfy {condition:?}"
                )));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    pub fn logs(&self, name: &str, options: &LogOptions<'_>) -> Result<()> {
        if self.sdk.existing(name)?.unwrap_or(false) {
            return Err(Error::Unsupported {
                service: name.to_owned(),
                feature: "logs are not exposed by the current WSLC SDK".to_owned(),
            });
        }
        self.inherit(&log_args(name, options))
    }

    pub fn logs_many(&self, services: &[(String, String)], options: &LogOptions<'_>) -> Result<()> {
        if services.len() == 1 {
            return self.logs(&services[0].1, options);
        }
        for (service, container) in services {
            if self.sdk.existing(container)?.unwrap_or(false) {
                return Err(Error::Unsupported {
                    service: service.clone(),
                    feature: "logs are not exposed by the current WSLC SDK".to_owned(),
                });
            }
        }

        let width = services
            .iter()
            .map(|(service, _)| service.len())
            .max()
            .unwrap_or_default();
        let stdout = Arc::new(Mutex::new(io::stdout()));
        let stderr = Arc::new(Mutex::new(io::stderr()));
        let mut children: Vec<(std::process::Child, Vec<String>)> =
            Vec::with_capacity(services.len());
        let mut readers = Vec::with_capacity(services.len() * 2);

        for (service, container) in services {
            let args = log_args(container, options);
            let mut child = match Command::new(&self.program)
                .args(&args)
                .current_dir(&self.working_dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(child) => child,
                Err(error) => {
                    for (running, _) in &mut children {
                        let _ = running.kill();
                    }
                    return Err(Error::StartWslc(error));
                }
            };
            let child_stdout = child.stdout.take().expect("piped stdout");
            let child_stderr = child.stderr.take().expect("piped stderr");
            let stdout = Arc::clone(&stdout);
            let stderr = Arc::clone(&stderr);
            let stdout_prefix = service.clone();
            let stderr_prefix = service.clone();
            readers.push(std::thread::spawn(move || {
                write_prefixed_lines(child_stdout, stdout, &stdout_prefix, width)
            }));
            readers.push(std::thread::spawn(move || {
                write_prefixed_lines(child_stderr, stderr, &stderr_prefix, width)
            }));
            children.push((child, args));
        }

        let mut command_failure = None;
        for (child, args) in &mut children {
            let status = child.wait().map_err(Error::StartWslc)?;
            if !status.success() && command_failure.is_none() {
                command_failure = Some(Error::WslcCommand {
                    command: args.join(" "),
                    code: status.code().unwrap_or(-1),
                    message: "see prefixed wslc output above".to_owned(),
                });
            }
        }
        for reader in readers {
            reader
                .join()
                .map_err(|_| Error::LogStream("log reader thread panicked".to_owned()))?
                .map_err(|error| Error::LogStream(error.to_string()))?;
        }
        command_failure.map_or(Ok(()), Err)
    }

    pub fn stats(&self, name: &str, all: bool, no_trunc: bool, json: bool) -> Result<()> {
        if self.sdk.existing(name)?.unwrap_or(false) {
            return Err(Error::Unsupported {
                service: name.to_owned(),
                feature: "stats are not exposed by the current WSLC SDK".to_owned(),
            });
        }
        self.inherit(&stats_args(name, all, no_trunc, json))
    }

    pub fn exec(&self, name: &str, options: ExecOptions<'_>) -> Result<()> {
        if let Some(output) =
            self.sdk
                .exec(name, options.command, options.environment, options.workdir)?
        {
            print!("{}", String::from_utf8_lossy(&output.stdout));
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            if output.status != 0 {
                return Err(Error::WslcCommand {
                    command: options.command.join(" "),
                    code: output.status,
                    message: "SDK container process exited unsuccessfully".to_owned(),
                });
            }
            return Ok(());
        }
        let mut args = vec!["exec".to_owned()];
        if options.detach {
            args.push("--detach".to_owned());
        }
        if options.interactive {
            args.push("--interactive".to_owned());
        }
        if options.tty {
            args.push("--tty".to_owned());
        }
        for value in options.environment {
            args.extend(["--env".to_owned(), value.clone()]);
        }
        if let Some(user) = options.user {
            args.extend(["--user".to_owned(), user.to_owned()]);
        }
        if let Some(workdir) = options.workdir {
            args.extend(["--workdir".to_owned(), workdir.to_owned()]);
        }
        args.push(name.to_owned());
        args.extend(options.command.iter().cloned());
        self.inherit(&args)
    }

    fn ensure_resource(&self, kind: &str, resource: &Resource, project: &str) -> Result<()> {
        let existing = self.capture(&[kind.to_owned(), "list".to_owned(), "--quiet".to_owned()])?;
        if existing.lines().any(|line| line.trim() == resource.name) {
            return Ok(());
        }
        if resource.external {
            return Err(Error::InvalidConfig(format!(
                "external {kind} does not exist: {}",
                resource.name
            )));
        }

        let mut args = vec![kind.to_owned(), "create".to_owned()];
        if let Some(driver) = &resource.driver {
            args.extend(["--driver".to_owned(), driver.clone()]);
        }
        args.extend([
            "--label".to_owned(),
            format!("com.docker.compose.project={project}"),
            "--label".to_owned(),
            format!("com.docker.compose.{kind}={}", resource.key),
        ]);
        for (key, value) in &resource.labels {
            args.extend(["--label".to_owned(), format!("{key}={value}")]);
        }
        args.push(resource.name.clone());
        self.inherit(&args)
    }

    fn remove_resource(&self, kind: &str, name: &str) -> Result<()> {
        let existing = self.capture(&[kind.to_owned(), "list".to_owned(), "--quiet".to_owned()])?;
        if !existing.lines().any(|line| line.trim() == name) {
            return Ok(());
        }
        self.inherit(&[
            kind.to_owned(),
            "remove".to_owned(),
            "--force".to_owned(),
            name.to_owned(),
        ])
    }

    fn list_project_containers(
        &self,
        project: &Project,
        all: bool,
    ) -> Result<Vec<ListedContainer>> {
        let output = self.capture(&project_list_args(project, all, false, true))?;
        serde_json::from_str(&output).map_err(Error::Json)
    }

    fn inspect_container_state(&self, name: &str) -> Result<InspectedContainerState> {
        Ok(self.inspect_container(name)?.state)
    }

    fn inspect_container(&self, name: &str) -> Result<InspectedContainer> {
        let output = self.capture(&[
            "inspect".to_owned(),
            "--type".to_owned(),
            "container".to_owned(),
            name.to_owned(),
        ])?;
        let mut containers: Vec<InspectedContainer> = serde_json::from_str(&output)?;
        containers.pop().ok_or_else(|| {
            Error::InvalidConfig(format!("wslc inspect returned no container for {name}"))
        })
    }

    fn capture(&self, args: &[String]) -> Result<String> {
        let output = Command::new(&self.program)
            .args(args)
            .current_dir(&self.working_dir)
            .output()
            .map_err(Error::StartWslc)?;
        if !output.status.success() {
            return Err(command_error(args, output.status, &output.stderr));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn status_quiet(&self, args: &[String]) -> Result<ExitStatus> {
        Command::new(&self.program)
            .args(args)
            .current_dir(&self.working_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(Error::StartWslc)
    }

    fn inherit(&self, args: &[String]) -> Result<()> {
        self.inherit_with_quiet(args, false)
    }

    fn inherit_with_quiet(&self, args: &[String], quiet: bool) -> Result<()> {
        let mut command = Command::new(&self.program);
        command.args(args).current_dir(&self.working_dir);
        if quiet {
            command.stdout(Stdio::null());
        }
        let status = command.status().map_err(Error::StartWslc)?;
        if !status.success() {
            return Err(Error::WslcCommand {
                command: args.join(" "),
                code: status.code().unwrap_or(-1),
                message: "see wslc output above".to_owned(),
            });
        }
        Ok(())
    }
    pub fn uses_sdk_project(&self, project: &Project) -> bool {
        SdkClient::project_uses_sdk(project)
    }
}

fn project_list_args(project: &Project, all: bool, quiet: bool, json: bool) -> Vec<String> {
    let mut args = vec!["list".to_owned()];
    if all {
        args.push("--all".to_owned());
    }
    args.extend([
        "--filter".to_owned(),
        format!("label=com.docker.compose.project={}", project.name),
    ]);
    if quiet {
        args.push("--quiet".to_owned());
    } else if json {
        args.extend(["--format".to_owned(), "json".to_owned()]);
    }
    args
}

fn truncate_id(id: &str) -> String {
    id.chars().take(12).collect()
}

fn container_state(state: i32) -> &'static str {
    match state {
        1 => "Created",
        2 => "Running",
        3 => "Exited",
        4 => "Deleted",
        _ => "Invalid",
    }
}

fn orphan_names(mut existing: Vec<String>, expected: &BTreeSet<&str>) -> Vec<String> {
    existing.sort();
    existing.dedup();
    existing
        .into_iter()
        .filter(|name| !expected.contains(name.as_str()))
        .collect()
}

fn dependency_wait_timeout() -> Result<Option<Duration>> {
    parse_dependency_wait_timeout(std::env::var(DEPENDENCY_WAIT_TIMEOUT_ENV).ok().as_deref())
}

pub(crate) fn service_config_hash(service: &Service) -> Result<String> {
    #[derive(Serialize)]
    struct RuntimeConfig<'a> {
        image: String,
        command: &'a [String],
        entrypoint: &'a [String],
        environment: &'a BTreeMap<String, String>,
        ports: &'a [String],
        mounts: &'a [Mount],
        labels: &'a BTreeMap<String, String>,
        networks: &'a [crate::model::ServiceNetwork],
        hostname: &'a Option<String>,
        domain_name: &'a Option<String>,
        privileged: bool,
        working_dir: &'a Option<String>,
        user: &'a Option<String>,
        tty: bool,
        stdin_open: bool,
        gpus: bool,
        memory: &'a Option<String>,
        cpus: &'a Option<String>,
        ulimits: &'a BTreeMap<String, String>,
        healthcheck: &'a Option<crate::model::Healthcheck>,
        stop_signal: &'a str,
    }

    let image = service
        .image
        .as_deref()
        .ok_or_else(|| Error::MissingImage {
            service: service.name.clone(),
        })?;
    let image = if service.build.is_some() {
        image.to_owned()
    } else {
        resolve_image_reference(image)?
    };
    let environment = service
        .environment
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let labels = service
        .labels
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let ulimits = service
        .ulimits
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let config = RuntimeConfig {
        image,
        command: &service.command,
        entrypoint: &service.entrypoint,
        environment: &environment,
        ports: &service.ports,
        mounts: &service.mounts,
        labels: &labels,
        networks: &service.networks,
        hostname: &service.hostname,
        domain_name: &service.domain_name,
        privileged: service.privileged,
        working_dir: &service.working_dir,
        user: &service.user,
        tty: service.tty,
        stdin_open: service.stdin_open,
        gpus: service.gpus,
        memory: &service.memory,
        cpus: &service.cpus,
        ulimits: &ulimits,
        healthcheck: &service.healthcheck,
        stop_signal: &service.stop_signal,
    };
    let encoded = serde_json::to_vec(&config)?;
    let digest = Sha256::digest(encoded);
    Ok(format!("sha256:{digest:x}"))
}

fn log_args(name: &str, options: &LogOptions<'_>) -> Vec<String> {
    let mut args = vec!["logs".to_owned()];
    if options.follow {
        args.push("--follow".to_owned());
    }
    if let Some(tail) = options.tail {
        args.extend(["--tail".to_owned(), tail.to_string()]);
    }
    if options.timestamps {
        args.push("--timestamps".to_owned());
    }
    if let Some(since) = options.since {
        args.extend(["--since".to_owned(), since.to_owned()]);
    }
    if let Some(until) = options.until {
        args.extend(["--until".to_owned(), until.to_owned()]);
    }
    args.push(name.to_owned());
    args
}

fn stats_args(name: &str, all: bool, no_trunc: bool, json: bool) -> Vec<String> {
    let mut args = vec!["stats".to_owned()];
    if all {
        args.push("--all".to_owned());
    }
    if no_trunc {
        args.push("--no-trunc".to_owned());
    }
    args.extend([
        "--format".to_owned(),
        if json { "json" } else { "table" }.to_owned(),
        name.to_owned(),
    ]);
    args
}

fn write_prefixed_lines<R, W>(
    reader: R,
    writer: Arc<Mutex<W>>,
    prefix: &str,
    width: usize,
) -> io::Result<()>
where
    R: io::Read,
    W: Write,
{
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        let mut writer = writer
            .lock()
            .map_err(|_| io::Error::other("log output lock poisoned"))?;
        write!(writer, "{prefix:width$} | ")?;
        writer.write_all(&line)?;
        writeln!(writer)?;
        writer.flush()?;
    }
    Ok(())
}

fn parse_dependency_wait_timeout(value: Option<&str>) -> Result<Option<Duration>> {
    let seconds = match value {
        Some(value) => value.parse::<u64>().map_err(|_| {
            Error::InvalidConfig(format!(
                "{DEPENDENCY_WAIT_TIMEOUT_ENV} must be a non-negative integer number of seconds"
            ))
        })?,
        None => DEFAULT_DEPENDENCY_WAIT_TIMEOUT_SECS,
    };
    Ok((seconds != 0).then(|| Duration::from_secs(seconds)))
}

fn command_error(args: &[String], status: ExitStatus, stderr: &[u8]) -> Error {
    Error::WslcCommand {
        command: args.join(" "),
        code: status.code().unwrap_or(-1),
        message: String::from_utf8_lossy(stderr).trim().to_owned(),
    }
}

fn create_args(
    project: &Project,
    service: &Service,
    name: &str,
    auto_remove: bool,
    include_ports: bool,
    command_override: &[String],
    extra_environment: &[String],
) -> Result<Vec<String>> {
    let image = service
        .image
        .as_deref()
        .ok_or_else(|| Error::MissingImage {
            service: service.name.clone(),
        })?;
    let mut args = vec!["create".to_owned(), "--name".to_owned(), name.to_owned()];

    if auto_remove {
        args.push("--rm".to_owned());
    }
    if let Some(hostname) = &service.hostname {
        args.extend(["--hostname".to_owned(), hostname.clone()]);
    }
    if let Some(domain_name) = &service.domain_name {
        args.extend(["--domainname".to_owned(), domain_name.clone()]);
    }
    if service.gpus {
        args.extend(["--gpus".to_owned(), "all".to_owned()]);
    }
    if let Some(memory) = &service.memory {
        args.extend(["--memory".to_owned(), memory.clone()]);
    }
    if let Some(cpus) = &service.cpus {
        args.extend(["--cpus".to_owned(), cpus.clone()]);
    }
    for (_, ulimit) in &service.ulimits {
        args.extend(["--ulimit".to_owned(), ulimit.clone()]);
    }
    if let Some(healthcheck) = &service.healthcheck {
        if healthcheck.disabled {
            args.push("--no-healthcheck".to_owned());
        } else {
            if let Some(command) = &healthcheck.command {
                args.extend(["--health-cmd".to_owned(), command.clone()]);
            }
            if let Some(interval) = &healthcheck.interval {
                args.extend(["--health-interval".to_owned(), interval.clone()]);
            }
            if let Some(timeout) = &healthcheck.timeout {
                args.extend(["--health-timeout".to_owned(), timeout.clone()]);
            }
            if let Some(start_period) = &healthcheck.start_period {
                args.extend(["--health-start-period".to_owned(), start_period.clone()]);
            }
            if let Some(retries) = healthcheck.retries {
                args.extend(["--health-retries".to_owned(), retries.to_string()]);
            }
        }
    }
    if let Some(workdir) = &service.working_dir {
        args.extend(["--workdir".to_owned(), workdir.clone()]);
    }
    if let Some(user) = &service.user {
        args.extend(["--user".to_owned(), user.clone()]);
    }
    if service.tty {
        args.push("--tty".to_owned());
    }
    if service.stdin_open {
        args.push("--interactive".to_owned());
    }
    args.extend(["--stop-signal".to_owned(), service.stop_signal.clone()]);

    for (key, value) in &service.environment {
        args.extend(["--env".to_owned(), format!("{key}={value}")]);
    }
    for value in extra_environment {
        args.extend(["--env".to_owned(), value.clone()]);
    }
    if include_ports {
        for port in &service.ports {
            args.extend(["--publish".to_owned(), port.clone()]);
        }
    }
    for mount in &service.mounts {
        match mount {
            Mount::Tmpfs { .. } => {
                args.extend(["--tmpfs".to_owned(), mount.as_cli_value()]);
            }
            _ => args.extend(["--volume".to_owned(), mount.as_cli_value()]),
        }
    }
    for network in &service.networks {
        args.extend(["--network".to_owned(), network.name.clone()]);
        args.extend(["--network-alias".to_owned(), service.name.clone()]);
        for alias in &network.aliases {
            args.extend(["--network-alias".to_owned(), alias.clone()]);
        }
    }

    let mut labels = service.labels.clone();
    labels.insert(
        "com.docker.compose.project".to_owned(),
        project.name.clone(),
    );
    labels.insert(
        "com.docker.compose.service".to_owned(),
        service.name.clone(),
    );
    labels.insert(CONFIG_HASH_LABEL.to_owned(), service_config_hash(service)?);
    labels.insert(
        "com.docker.compose.oneoff".to_owned(),
        auto_remove.to_string(),
    );
    labels.insert(
        "com.docker.compose.project.working_dir".to_owned(),
        project.working_dir.display().to_string(),
    );
    labels.insert(
        "com.docker.compose.project.config_files".to_owned(),
        project
            .source_files
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(","),
    );
    for (key, value) in labels {
        args.extend(["--label".to_owned(), format!("{key}={value}")]);
    }

    if let Some(entrypoint) = service.entrypoint.first() {
        args.extend(["--entrypoint".to_owned(), entrypoint.clone()]);
    }
    args.push(if service.build.is_some() {
        image.to_owned()
    } else {
        resolve_image_reference(image)?
    });

    if service.entrypoint.len() > 1 {
        args.extend(service.entrypoint.iter().skip(1).cloned());
    }
    if command_override.is_empty() {
        args.extend(service.command.iter().cloned());
    } else {
        args.extend(command_override.iter().cloned());
    }
    Ok(args)
}

pub(crate) fn resolve_image_reference(image: &str) -> Result<String> {
    resolve_image_reference_with(image, |key| std::env::var(key).ok())
}

fn resolve_image_reference_with<F>(image: &str, mut env: F) -> Result<String>
where
    F: FnMut(&str) -> Option<String>,
{
    let (registry, remainder) = split_registry(image);
    let env_key = if registry == "docker.io" {
        "WSLC_REGISTRY_MIRROR".to_owned()
    } else {
        format!(
            "WSLC_REGISTRY_MIRROR_{}",
            registry
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_uppercase()
                } else {
                    '_'
                })
                .collect::<String>()
        )
    };
    let Some(mirror) = env(&env_key) else {
        return Ok(image.to_owned());
    };
    let mirror = mirror.trim().trim_end_matches('/');
    if mirror.is_empty() || mirror.contains("://") {
        return Err(Error::InvalidConfig(format!(
            "{env_key} must contain a registry host, not a URL"
        )));
    }
    Ok(format!("{mirror}/{remainder}"))
}

fn split_registry(image: &str) -> (&str, &str) {
    let first = image.split('/').next().unwrap_or(image);
    if image.contains('/') && (first.contains('.') || first.contains(':') || first == "localhost") {
        let remainder = image
            .strip_prefix(first)
            .unwrap_or(image)
            .trim_start_matches('/');
        let registry = if matches!(first, "index.docker.io" | "registry-1.docker.io") {
            "docker.io"
        } else {
            first
        };
        (registry, remainder)
    } else {
        (
            "docker.io",
            image.strip_prefix("docker.io/").unwrap_or(image),
        )
    }
}

#[cfg(test)]
mod tests {
    use indexmap::{IndexMap, IndexSet};

    use super::*;
    use crate::model::{Healthcheck, Service, ServiceNetwork};

    fn project_and_service() -> (Project, Service) {
        let service = Service {
            name: "web".to_owned(),
            image: Some("docker.io/library/alpine:latest".to_owned()),
            build: None,
            container_name: "demo-web-1".to_owned(),
            command: vec!["sleep".to_owned(), "60".to_owned()],
            entrypoint: Vec::new(),
            environment: IndexMap::from([("MODE".to_owned(), "test".to_owned())]),
            ports: vec!["8080:80".to_owned()],
            mounts: Vec::new(),
            depends_on: Vec::new(),
            dependency_conditions: IndexMap::new(),
            profiles: IndexSet::new(),
            labels: IndexMap::new(),
            networks: vec![ServiceNetwork {
                name: "demo_default".to_owned(),
                aliases: Vec::new(),
            }],
            hostname: None,
            domain_name: None,
            privileged: false,
            working_dir: None,
            user: None,
            tty: false,
            stdin_open: false,
            gpus: false,
            memory: None,
            cpus: None,
            ulimits: IndexMap::new(),
            healthcheck: None,
            stop_signal: "SIGTERM".to_owned(),
            stop_grace_period: std::time::Duration::from_secs(10),
            restart: None,
            unsupported: Vec::new(),
        };
        let project = Project {
            name: "demo".to_owned(),
            working_dir: PathBuf::from(r"C:\demo"),
            source_files: vec![PathBuf::from(r"C:\demo\compose.yaml")],
            services: IndexMap::from([("web".to_owned(), service.clone())]),
            networks: IndexMap::new(),
            volumes: IndexMap::new(),
        };
        (project, service)
    }

    #[test]
    fn create_arguments_include_compose_identity_and_runtime_options() {
        let (project, mut service) = project_and_service();
        service.healthcheck = Some(Healthcheck {
            command: Some("wget -q localhost/health".to_owned()),
            interval: Some("5s".to_owned()),
            timeout: Some("2s".to_owned()),
            start_period: Some("10s".to_owned()),
            retries: Some(3),
            disabled: false,
        });
        let args = create_args(
            &project,
            &service,
            &service.container_name,
            false,
            true,
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(args[0], "create");
        assert!(args.windows(2).any(|pair| pair == ["--name", "demo-web-1"]));
        assert!(args.windows(2).any(|pair| pair == ["--publish", "8080:80"]));
        assert!(args
            .iter()
            .any(|value| value == "com.docker.compose.project=demo"));
        assert!(args
            .iter()
            .any(|value| value.starts_with("com.docker.compose.config-hash=sha256:")));
        assert!(args.iter().any(|value| value == "MODE=test"));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--health-cmd", "wget -q localhost/health"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--health-retries", "3"]));
    }

    #[test]
    fn mirror_configuration_rejects_urls() {
        let result = resolve_image_reference_with("alpine:latest", |key| {
            (key == "WSLC_REGISTRY_MIRROR").then(|| "https://mirror.example.com".to_owned())
        });
        assert!(result.unwrap_err().to_string().contains("not a URL"));
    }

    #[test]
    fn project_list_arguments_apply_compose_label_and_output_flags() {
        let (project, _) = project_and_service();
        assert_eq!(
            project_list_args(&project, true, false, true),
            [
                "list",
                "--all",
                "--filter",
                "label=com.docker.compose.project=demo",
                "--format",
                "json",
            ]
        );
        assert_eq!(
            project_list_args(&project, false, true, false),
            [
                "list",
                "--filter",
                "label=com.docker.compose.project=demo",
                "--quiet",
            ]
        );
    }

    #[test]
    fn listed_container_preserves_wslc_json_fields() {
        let json = r#"[{"Id":"abcdef1234567890","Name":"demo-web-1","Image":"alpine","State":2,"Ports":[]}]"#;
        let containers: Vec<ListedContainer> = serde_json::from_str(json).unwrap();
        assert_eq!(containers[0].name, "demo-web-1");
        assert_eq!(truncate_id(&containers[0].id), "abcdef123456");
        assert_eq!(container_state(containers[0].state), "Running");
        assert!(containers[0].extra.contains_key("Ports"));
    }

    #[test]
    fn orphan_detection_is_exact_deterministic_and_deduplicated() {
        let expected = BTreeSet::from(["demo-web-1"]);
        assert_eq!(
            orphan_names(
                vec![
                    "demo-old-1".to_owned(),
                    "demo-web-1".to_owned(),
                    "demo-old-1".to_owned(),
                ],
                &expected,
            ),
            ["demo-old-1"]
        );
    }

    #[test]
    fn parses_wslc_dependency_runtime_state() {
        let json = r#"[{"State":{"Status":"running","Running":true,"ExitCode":0,"Health":{"Status":"healthy"}}}]"#;
        let containers: Vec<InspectedContainer> = serde_json::from_str(json).unwrap();
        assert_eq!(containers[0].state.status, "running");
        assert!(containers[0].state.running);
        assert_eq!(containers[0].state.exit_code, 0);
        assert_eq!(
            containers[0].state.health.as_ref().unwrap().status,
            "healthy"
        );
    }

    #[test]
    fn dependency_wait_timeout_defaults_and_can_be_disabled() {
        assert_eq!(
            parse_dependency_wait_timeout(None).unwrap(),
            Some(Duration::from_secs(120))
        );
        assert_eq!(parse_dependency_wait_timeout(Some("0")).unwrap(), None);
        assert!(parse_dependency_wait_timeout(Some("later")).is_err());
    }

    #[test]
    fn service_config_hash_is_stable_and_tracks_runtime_changes() {
        let (_, mut service) = project_and_service();
        let original = service_config_hash(&service).unwrap();
        assert_eq!(original, service_config_hash(&service).unwrap());
        assert_eq!(original.len(), "sha256:".len() + 64);

        service.command.push("changed".to_owned());
        assert_ne!(original, service_config_hash(&service).unwrap());
    }

    #[test]
    fn service_config_hash_ignores_mapping_and_dependency_order() {
        let (_, mut service) = project_and_service();
        service.environment = IndexMap::from([
            ("FIRST".to_owned(), "1".to_owned()),
            ("SECOND".to_owned(), "2".to_owned()),
        ]);
        service.depends_on = vec!["db".to_owned(), "cache".to_owned()];
        let original = service_config_hash(&service).unwrap();

        service.environment = IndexMap::from([
            ("SECOND".to_owned(), "2".to_owned()),
            ("FIRST".to_owned(), "1".to_owned()),
        ]);
        service.depends_on.reverse();
        assert_eq!(original, service_config_hash(&service).unwrap());
    }

    #[test]
    fn log_arguments_preserve_all_filters() {
        let args = log_args(
            "demo-web-1",
            &LogOptions {
                follow: true,
                tail: Some(20),
                timestamps: true,
                since: Some("100"),
                until: Some("200"),
            },
        );
        assert_eq!(
            args,
            [
                "logs",
                "--follow",
                "--tail",
                "20",
                "--timestamps",
                "--since",
                "100",
                "--until",
                "200",
                "demo-web-1",
            ]
        );
    }

    #[test]
    fn stats_arguments_select_exact_container_and_output_options() {
        assert_eq!(
            stats_args("demo-web-1", true, true, true),
            [
                "stats",
                "--all",
                "--no-trunc",
                "--format",
                "json",
                "demo-web-1",
            ]
        );
    }

    #[test]
    fn prefixes_each_log_line_for_multiplexed_output() {
        let output = Arc::new(Mutex::new(Vec::new()));
        write_prefixed_lines("one\ntwo\n".as_bytes(), Arc::clone(&output), "web", 6).unwrap();
        assert_eq!(
            String::from_utf8(output.lock().unwrap().clone()).unwrap(),
            "web    | one\nweb    | two\n"
        );
    }
}
