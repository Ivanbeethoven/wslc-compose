use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::model::{Mount, Project, Service};
use crate::{Error, Result};

const DAEMON_ARGUMENT: &str = "__sdk-daemon";
const SDK_TIMEOUT_ENV: &str = "WSLC_COMPOSE_SDK_TIMEOUT_SECS";
const DEFAULT_SDK_TIMEOUT_SECS: u64 = 60 * 60;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DaemonRecord {
    port: u16,
    token: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Request {
    token: String,
    command: CommandRequest,
}

#[derive(Debug, Deserialize, Serialize)]
struct Response {
    ok: bool,
    payload: serde_json::Value,
    error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum CommandRequest {
    Ping,
    Create {
        project: String,
        service: SdkService,
    },
    Exists {
        name: String,
    },
    Running {
        name: String,
    },
    Start {
        name: String,
    },
    Stop {
        name: String,
        signal: String,
        timeout: u64,
    },
    Remove {
        name: String,
        force: bool,
    },
    Exec {
        name: String,
        command: Vec<String>,
        environment: Vec<String>,
        workdir: Option<String>,
    },
    List {
        project: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SdkService {
    service_name: String,
    image: String,
    container_name: String,
    command: Vec<String>,
    entrypoint: Vec<String>,
    environment: Vec<(String, String)>,
    ports: Vec<String>,
    mounts: Vec<SdkMount>,
    hostname: Option<String>,
    domain_name: Option<String>,
    privileged: bool,
    working_dir: Option<String>,
    gpus: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SdkMount {
    source: PathBuf,
    target: String,
    read_only: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ContainerInfo {
    pub(crate) name: String,
    pub(crate) state: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ExecOutput {
    pub(crate) status: i32,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

pub struct Client {
    root: PathBuf,
}

impl Client {
    pub fn new() -> Self {
        Self { root: state_root() }
    }

    pub fn project_uses_sdk(project: &Project) -> bool {
        project.services.values().any(|service| service.privileged)
    }

    pub fn create(&self, project: &Project, service: &Service) -> Result<()> {
        let service = SdkService::from_service(project, service, &self.root)?;
        self.call_or_start(CommandRequest::Create {
            project: project.name.clone(),
            service,
        })?;
        Ok(())
    }

    pub fn existing(&self, name: &str) -> Result<Option<bool>> {
        self.call_existing(CommandRequest::Exists {
            name: name.to_owned(),
        })
    }

    pub fn running(&self, name: &str) -> Result<Option<bool>> {
        self.call_existing(CommandRequest::Running {
            name: name.to_owned(),
        })
    }

    pub fn start(&self, name: &str) -> Result<bool> {
        self.call_existing(CommandRequest::Start {
            name: name.to_owned(),
        })
        .map(|value| value.unwrap_or(false))
    }

    pub fn stop(&self, name: &str, signal: &str, timeout: u64) -> Result<bool> {
        self.call_existing(CommandRequest::Stop {
            name: name.to_owned(),
            signal: signal.to_owned(),
            timeout,
        })
        .map(|value| value.unwrap_or(false))
    }

    pub fn remove(&self, name: &str, force: bool) -> Result<bool> {
        self.call_existing(CommandRequest::Remove {
            name: name.to_owned(),
            force,
        })
        .map(|value| value.unwrap_or(false))
    }

    pub fn exec(
        &self,
        name: &str,
        command: &[String],
        environment: &[String],
        workdir: Option<&str>,
    ) -> Result<Option<ExecOutput>> {
        let Some(payload) = self.call_existing_raw(CommandRequest::Exec {
            name: name.to_owned(),
            command: command.to_vec(),
            environment: environment.to_vec(),
            workdir: workdir.map(str::to_owned),
        })?
        else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_value(payload)?))
    }

    pub fn list(&self, project: &str) -> Result<Option<Vec<ContainerInfo>>> {
        let Some(payload) = self.call_existing_raw(CommandRequest::List {
            project: project.to_owned(),
        })?
        else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_value(payload)?))
    }

    fn call_or_start(&self, command: CommandRequest) -> Result<serde_json::Value> {
        if let Some(response) = self.call_existing_raw(command_for_probe(&command))? {
            drop(response);
            return self.call_existing_raw(command)?.ok_or_else(|| {
                Error::InvalidConfig("SDK daemon stopped while handling a request".to_owned())
            });
        }
        self.start_daemon()?;
        self.call_existing_raw(command)?.ok_or_else(|| {
            Error::InvalidConfig("SDK daemon did not accept a request after startup".to_owned())
        })
    }

    fn call_existing(&self, command: CommandRequest) -> Result<Option<bool>> {
        let Some(payload) = self.call_existing_raw(command)? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_value(payload)?))
    }

    fn call_existing_raw(&self, command: CommandRequest) -> Result<Option<serde_json::Value>> {
        let Some(record) = self.read_live_record()? else {
            return Ok(None);
        };
        let response = send_request(&record, command)?;
        if !response.ok {
            return Err(Error::InvalidConfig(
                response
                    .error
                    .unwrap_or_else(|| "SDK daemon request failed".to_owned()),
            ));
        }
        Ok(Some(response.payload))
    }

    fn read_live_record(&self) -> Result<Option<DaemonRecord>> {
        let path = self.record_path();
        let Ok(contents) = fs::read(&path) else {
            return Ok(None);
        };
        let Ok(record) = serde_json::from_slice::<DaemonRecord>(&contents) else {
            let _ = fs::remove_file(path);
            return Ok(None);
        };
        match send_request(&record, CommandRequest::Ping) {
            Ok(response) if response.ok => Ok(Some(record)),
            _ => {
                let _ = fs::remove_file(self.record_path());
                Ok(None)
            }
        }
    }

    fn start_daemon(&self) -> Result<()> {
        fs::create_dir_all(&self.root).map_err(|error| {
            Error::InvalidConfig(format!(
                "failed to create SDK daemon state directory: {error}"
            ))
        })?;
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| {
            Error::InvalidConfig(format!("failed to reserve SDK daemon port: {error}"))
        })?;
        let port = listener
            .local_addr()
            .map_err(|error| Error::InvalidConfig(format!("invalid SDK daemon address: {error}")))?
            .port();
        drop(listener);

        let token = daemon_token();
        let executable = std::env::current_exe().map_err(|error| {
            Error::InvalidConfig(format!("failed to locate wslc-compose executable: {error}"))
        })?;
        Command::new(executable)
            .arg(DAEMON_ARGUMENT)
            .arg("--port")
            .arg(port.to_string())
            .arg("--token")
            .arg(&token)
            .arg("--state-root")
            .arg(&self.root)
            .spawn()
            .map_err(|error| {
                Error::InvalidConfig(format!("failed to start SDK daemon: {error}"))
            })?;

        let record = DaemonRecord { port, token };
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(100));
            if matches!(
                send_request(&record, CommandRequest::Ping),
                Ok(Response { ok: true, .. })
            ) {
                return Ok(());
            }
        }
        Err(Error::InvalidConfig(
            "SDK daemon did not become ready within three seconds".to_owned(),
        ))
    }

    fn record_path(&self) -> PathBuf {
        self.root.join("sdk-daemon.json")
    }
}

impl SdkService {
    fn from_service(project: &Project, service: &Service, state_root: &Path) -> Result<Self> {
        let image = service.image.clone().ok_or_else(|| Error::MissingImage {
            service: service.name.clone(),
        })?;
        let mut mounts = Vec::new();
        for mount in &service.mounts {
            match mount {
                Mount::Bind {
                    source,
                    target,
                    read_only,
                } => mounts.push(SdkMount {
                    source: source.clone(),
                    target: target.clone(),
                    read_only: *read_only,
                }),
                Mount::Volume {
                    source,
                    target,
                    read_only,
                } => mounts.push(SdkMount {
                    source: state_root.join("volumes").join(&project.name).join(source),
                    target: target.clone(),
                    read_only: *read_only,
                }),
                Mount::Anonymous { target, .. } | Mount::Tmpfs { target } => {
                    return Err(Error::Unsupported {
                        service: service.name.clone(),
                        feature: format!(
                            "SDK backend does not yet support mount type for {target}"
                        ),
                    });
                }
            }
        }
        Ok(Self {
            service_name: service.name.clone(),
            image,
            container_name: service.container_name.clone(),
            command: service.command.clone(),
            entrypoint: service.entrypoint.clone(),
            environment: service
                .environment
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            ports: service.ports.clone(),
            mounts,
            hostname: service.hostname.clone(),
            domain_name: service.domain_name.clone(),
            privileged: service.privileged,
            working_dir: service.working_dir.clone(),
            gpus: service.gpus,
        })
    }
}

pub fn run_if_requested() -> Result<bool> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some(DAEMON_ARGUMENT) {
        return Ok(false);
    }
    let port = daemon_arg(&args, "--port")?
        .parse::<u16>()
        .map_err(|error| Error::InvalidConfig(format!("invalid SDK daemon port: {error}")))?;
    let token = daemon_arg(&args, "--token")?.to_owned();
    let state_root = PathBuf::from(daemon_arg(&args, "--state-root")?);
    serve(port, token, state_root)?;
    Ok(true)
}

fn daemon_arg<'a>(args: &'a [String], name: &str) -> Result<&'a str> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
        .ok_or_else(|| Error::InvalidConfig(format!("missing {name} for SDK daemon")))
}

fn serve(port: u16, token: String, state_root: PathBuf) -> Result<()> {
    fs::create_dir_all(&state_root).map_err(|error| {
        Error::InvalidConfig(format!(
            "failed to create SDK daemon state directory: {error}"
        ))
    })?;
    let record_path = state_root.join("sdk-daemon.json");
    fs::write(
        &record_path,
        serde_json::to_vec(&DaemonRecord {
            port,
            token: token.clone(),
        })?,
    )
    .map_err(|error| {
        Error::InvalidConfig(format!("failed to publish SDK daemon endpoint: {error}"))
    })?;

    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|error| {
        Error::InvalidConfig(format!("failed to start SDK daemon listener: {error}"))
    })?;
    let mut state = DaemonState::new(state_root);
    let token = std::sync::Arc::new(token);
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let _ = handle_connection(stream, &token, &mut state);
    }
    Ok(())
}

struct DaemonState {
    root: PathBuf,
    session: Option<wslc::Session>,
    containers: IndexMap<String, wslc::Container>,
    projects: BTreeMap<String, String>,
    service_addresses: BTreeMap<(String, String), String>,
}

impl DaemonState {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            session: None,
            containers: IndexMap::new(),
            projects: BTreeMap::new(),
            service_addresses: BTreeMap::new(),
        }
    }

    fn session(&mut self) -> Result<&wslc::Session> {
        if self.session.is_none() {
            let storage = self.root.join("session");
            fs::create_dir_all(&storage).map_err(|error| {
                Error::InvalidConfig(format!("failed to create SDK session storage: {error}"))
            })?;
            self.session = Some(
                wslc::Session::builder(format!("wslc-compose-{}", std::process::id()), storage)
                    .terminate_on_drop(false)
                    .start()
                    .map_err(|error| {
                        Error::InvalidConfig(format!("SDK session creation failed: {error}"))
                    })?,
            );
        }
        Ok(self.session.as_ref().expect("session initialized"))
    }

    fn create(&mut self, project: String, mut service: SdkService) -> Result<()> {
        if self.containers.contains_key(&service.container_name) {
            return Ok(());
        }
        for mount in &service.mounts {
            fs::create_dir_all(&mount.source).map_err(|error| {
                Error::InvalidConfig(format!(
                    "failed to create SDK mount source {}: {error}",
                    mount.source.display()
                ))
            })?;
        }
        let session = self.session()?.clone();
        if let Err(error) = session
            .pull_image(wslc::ImagePullOptions::new(&service.image))
            .run()
        {
            eprintln!(
                "SDK daemon: image pull for {} skipped (already cached or unavailable): {error}",
                service.image
            );
        }

        self.resolve_service_urls(&project, &mut service.environment);

        let mut builder = session
            .container(wslc::ContainerOptions::new(&service.image))
            .name(&service.container_name)
            .privileged(service.privileged)
            .enable_gpu(service.gpus);
        if let Some(hostname) = service.hostname {
            builder = builder.hostname(hostname);
        }
        if let Some(domain_name) = service.domain_name {
            builder = builder.domain_name(domain_name);
        }
        let mut cmdline = service.entrypoint;
        if cmdline.is_empty() {
            cmdline = service.command;
        } else {
            cmdline.extend(service.command);
        }
        cmdline.retain(|arg| !arg.is_empty());
        if !cmdline.is_empty() {
            let mut process = wslc::ProcessOptions::new(cmdline);
            if let Some(workdir) = service.working_dir {
                process = process.working_dir(workdir);
            }
            for (key, value) in service.environment {
                process = process.env(key, value);
            }
            builder = builder.init_process(process);
        }
        for port in &service.ports {
            let (host, container) = parse_port(port)?;
            builder = builder.port(host, container);
        }
        for mount in service.mounts {
            builder = if mount.read_only {
                builder.volume_read_only(mount.source, mount.target)
            } else {
                builder.volume(mount.source, mount.target)
            };
        }
        let container = builder.create().map_err(|error| {
            Error::InvalidConfig(format!("SDK container creation failed: {error}"))
        })?;
        container.start().map_err(|error| {
            Error::InvalidConfig(format!("SDK container start failed: {error}"))
        })?;
        let address = container_address(&container)?;
        self.projects
            .insert(service.container_name.clone(), project.clone());
        self.service_addresses
            .insert((project, service.service_name), address);
        self.containers.insert(service.container_name, container);
        Ok(())
    }

    fn resolve_service_urls(&self, project: &str, environment: &mut [(String, String)]) {
        for (key, value) in environment {
            for ((address_project, service), address) in &self.service_addresses {
                if address_project != project {
                    continue;
                }
                let scheme_host = format!("://{service}");
                if value.contains(&scheme_host) {
                    *value = value.replace(&scheme_host, &format!("://{address}"));
                    eprintln!("SDK network: resolved {service} for {key}");
                }
            }
        }
    }
}

fn container_address(container: &wslc::Container) -> Result<String> {
    let inspect = container.inspect().map_err(Error::Wslc)?;
    let value: serde_json::Value = serde_json::from_str(&inspect)?;
    find_ipv4_address(&value).ok_or_else(|| {
        Error::InvalidConfig(format!(
            "SDK container inspect did not include an IPv4 address: {inspect}"
        ))
    })
}

fn find_ipv4_address(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(key.as_str(), "IPAddress" | "ip_address" | "ipAddress") {
                    if let Some(address) = value
                        .as_str()
                        .filter(|address| address.parse::<std::net::Ipv4Addr>().is_ok())
                    {
                        return Some(address.to_owned());
                    }
                }
                if let Some(address) = find_ipv4_address(value) {
                    return Some(address);
                }
            }
            None
        }
        serde_json::Value::Array(values) => values.iter().find_map(find_ipv4_address),
        _ => None,
    }
}

fn handle_connection(mut stream: TcpStream, token: &str, state: &mut DaemonState) -> Result<()> {
    let request = {
        let mut reader = BufReader::new(&mut stream);
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|error| {
            Error::InvalidConfig(format!("failed to read SDK daemon request: {error}"))
        })?;
        serde_json::from_str::<Request>(&line)?
    };
    let response = if request.token != token {
        Response {
            ok: false,
            payload: serde_json::Value::Null,
            error: Some("SDK daemon authentication failed".to_owned()),
        }
    } else {
        match dispatch(request.command, state) {
            Ok(payload) => Response {
                ok: true,
                payload,
                error: None,
            },
            Err(error) => Response {
                ok: false,
                payload: serde_json::Value::Null,
                error: Some(error.to_string()),
            },
        }
    };
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n").map_err(|error| {
        Error::InvalidConfig(format!("failed to write SDK daemon response: {error}"))
    })?;
    Ok(())
}

fn dispatch(command: CommandRequest, state: &mut DaemonState) -> Result<serde_json::Value> {
    match command {
        CommandRequest::Ping => Ok(serde_json::Value::Bool(true)),
        CommandRequest::Create { project, service } => {
            state.create(project, service)?;
            Ok(serde_json::Value::Bool(true))
        }
        CommandRequest::Exists { name } => Ok(serde_json::Value::Bool(
            state.containers.contains_key(&name),
        )),
        CommandRequest::Running { name } => {
            let running = state
                .containers
                .get(&name)
                .map(|container| {
                    container
                        .state()
                        .map(|state| state == wslc::ContainerState::Running)
                })
                .transpose()
                .map_err(Error::Wslc)?
                .unwrap_or(false);
            Ok(serde_json::Value::Bool(running))
        }
        CommandRequest::Start { name } => {
            let Some(container) = state.containers.get(&name) else {
                return Ok(serde_json::Value::Bool(false));
            };
            if container.state().map_err(Error::Wslc)? != wslc::ContainerState::Running {
                container.start().map_err(Error::Wslc)?;
            }
            Ok(serde_json::Value::Bool(true))
        }
        CommandRequest::Stop {
            name,
            signal,
            timeout,
        } => {
            let Some(container) = state.containers.get(&name) else {
                return Ok(serde_json::Value::Bool(false));
            };
            container
                .stop(parse_signal(&signal), Duration::from_secs(timeout))
                .map_err(Error::Wslc)?;
            Ok(serde_json::Value::Bool(true))
        }
        CommandRequest::Remove { name, force } => {
            let Some(container) = state.containers.shift_remove(&name) else {
                return Ok(serde_json::Value::Bool(false));
            };
            state.projects.remove(&name);
            container
                .delete(wslc::DeleteContainerOptions::default().force(force))
                .map_err(Error::Wslc)?;
            Ok(serde_json::Value::Bool(true))
        }
        CommandRequest::Exec {
            name,
            command,
            environment,
            workdir,
        } => {
            let Some(container) = state.containers.get(&name) else {
                return Err(Error::InvalidConfig(format!(
                    "SDK container not found: {name}"
                )));
            };
            let mut process = wslc::ProcessOptions::new(command).capture_stdout();
            if let Some(workdir) = workdir {
                process = process.working_dir(workdir);
            }
            for value in environment {
                let Some((key, value)) = value.split_once('=') else {
                    return Err(Error::InvalidConfig(format!(
                        "invalid exec environment value: {value}"
                    )));
                };
                process = process.env(key, value);
            }
            let output = container
                .exec(process)
                .map_err(Error::Wslc)?
                .wait_with_output()
                .map_err(Error::Wslc)?;
            serde_json::to_value(ExecOutput {
                status: output.status,
                stdout: output.stdout,
                stderr: output.stderr,
            })
            .map_err(Error::Json)
        }
        CommandRequest::List { project } => {
            let mut containers = Vec::new();
            for (name, container) in &state.containers {
                if state
                    .projects
                    .get(name)
                    .is_some_and(|value| value == &project)
                {
                    containers.push(ContainerInfo {
                        name: name.clone(),
                        state: format!("{:?}", container.state().map_err(Error::Wslc)?),
                    });
                }
            }
            serde_json::to_value(containers).map_err(Error::Json)
        }
    }
}

fn parse_port(value: &str) -> Result<(u16, u16)> {
    let (host, container) = value.rsplit_once(':').ok_or_else(|| {
        Error::InvalidConfig(format!(
            "SDK backend requires explicit HOST:CONTAINER port mapping: {value}"
        ))
    })?;
    let host = host.rsplit(':').next().unwrap_or(host);
    let container = container.split('/').next().unwrap_or(container);
    let host = host
        .parse::<u16>()
        .map_err(|_| Error::InvalidConfig(format!("invalid host port: {value}")))?;
    let container = container
        .parse::<u16>()
        .map_err(|_| Error::InvalidConfig(format!("invalid container port: {value}")))?;
    if host == 0 || container == 0 {
        return Err(Error::InvalidConfig(format!(
            "SDK backend does not support ephemeral port mappings: {value}"
        )));
    }
    Ok((host, container))
}

fn parse_signal(signal: &str) -> wslc::Signal {
    match signal {
        "SIGHUP" => wslc::Signal::Sighup,
        "SIGINT" => wslc::Signal::Sigint,
        "SIGKILL" => wslc::Signal::Sigkill,
        _ => wslc::Signal::Sigterm,
    }
}

fn send_request(record: &DaemonRecord, command: CommandRequest) -> Result<Response> {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", record.port)
            .parse()
            .map_err(|error| {
                Error::InvalidConfig(format!("invalid SDK daemon address: {error}"))
            })?,
        Duration::from_secs(30),
    )
    .map_err(|error| Error::InvalidConfig(format!("SDK daemon is unavailable: {error}")))?;
    stream
        .set_read_timeout(sdk_response_timeout())
        .map_err(|error| {
            Error::InvalidConfig(format!("failed to set SDK daemon read timeout: {error}"))
        })?;
    serde_json::to_writer(
        &mut stream,
        &Request {
            token: record.token.clone(),
            command,
        },
    )?;
    stream.write_all(b"\n").map_err(|error| {
        Error::InvalidConfig(format!("failed to write SDK daemon request: {error}"))
    })?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|error| {
            Error::InvalidConfig(format!("failed to read SDK daemon response: {error}"))
        })?;
    serde_json::from_str(&response).map_err(Error::Json)
}

fn sdk_response_timeout() -> Option<Duration> {
    parse_sdk_response_timeout(std::env::var(SDK_TIMEOUT_ENV).ok().as_deref())
}

fn parse_sdk_response_timeout(value: Option<&str>) -> Option<Duration> {
    match value.and_then(|value| value.parse::<u64>().ok()) {
        Some(0) => None,
        Some(seconds) => Some(Duration::from_secs(seconds)),
        None => Some(Duration::from_secs(DEFAULT_SDK_TIMEOUT_SECS)),
    }
}

fn command_for_probe(command: &CommandRequest) -> CommandRequest {
    match command {
        CommandRequest::Create { .. } => CommandRequest::Ping,
        _ => CommandRequest::Ping,
    }
}

fn state_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("wslc-compose")
}

fn daemon_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:032x}{:08x}", nanos, std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nested_container_ipv4_address() {
        let inspect = serde_json::json!({
            "NetworkSettings": { "Networks": { "default": { "IPAddress": "172.17.0.2" } } }
        });
        assert_eq!(find_ipv4_address(&inspect).as_deref(), Some("172.17.0.2"));
    }

    #[test]
    fn parses_ip_bound_port_mappings() {
        assert_eq!(
            parse_port("127.0.0.1:26379:6379/tcp").unwrap(),
            (26379, 6379)
        );
    }

    #[test]
    fn replaces_service_urls_with_sdk_addresses() {
        let mut state = DaemonState::new(PathBuf::from("C:/sdk"));
        state.service_addresses.insert(
            ("demo".to_owned(), "redis".to_owned()),
            "172.17.0.2".to_owned(),
        );
        let mut environment = vec![("REDIS_URL".to_owned(), "redis://redis:6379/0".to_owned())];
        state.resolve_service_urls("demo", &mut environment);
        assert_eq!(environment[0].1, "redis://172.17.0.2:6379/0");
    }

    #[test]
    fn sdk_response_timeout_defaults_to_one_hour() {
        assert_eq!(
            parse_sdk_response_timeout(None),
            Some(Duration::from_secs(60 * 60))
        );
        assert_eq!(
            parse_sdk_response_timeout(Some("invalid")),
            Some(Duration::from_secs(60 * 60))
        );
    }

    #[test]
    fn sdk_response_timeout_accepts_seconds_and_zero_disables_it() {
        assert_eq!(
            parse_sdk_response_timeout(Some("900")),
            Some(Duration::from_secs(900))
        );
        assert_eq!(parse_sdk_response_timeout(Some("0")), None);
    }
}
