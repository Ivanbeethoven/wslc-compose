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
        let Some(mut record) = Self::read_record(&self.root)? else {
            return Ok(None);
        };
        let response = send_request(&record, command)?;
        if !response.ok {
            Self::delete_record(&self.root)?;
            return Ok(None);
        }
        Ok(Some(response.payload))
    }

    fn read_record(root: &Path) -> Result<Option<DaemonRecord>> {
        let path = root.join("sdk-daemon.json");
        match fs::read_to_string(&path) {
            Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Error::InvalidConfig(format!(
                "failed to read SDK daemon record: {error}"
            ))),
        }
    }

    fn delete_record(root: &Path) -> Result<()> {
        let path = root.join("sdk-daemon.json");
        fs::remove_file(&path).map_err(|error| {
            Error::InvalidConfig(format!("failed to remove SDK daemon record: {error}"))
        })
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

        let record_path = self.root.join("sdk-daemon.json");
        std::thread::sleep(Duration::from_millis(100));
        for _ in 0..30 {
            if record_path.exists() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(Error::InvalidConfig(
            "SDK daemon did not publish its endpoint".to_owned(),
        ))
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
    let state = std::sync::Arc::new(std::sync::Mutex::new(DaemonState::new(state_root)));
    let token = std::sync::Arc::new(token);
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let state = state.clone();
        let token = token.clone();
        std::thread::spawn(move || {
            let _ = handle_connection(stream, &token, &mut state.lock().unwrap());
        });
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
            fs::create_dir_all(&storage).ok();
            self.session = Some(
                wslc::Session::builder("wslc-compose", storage)
                    .terminate_on_drop(false)
                    .start()
                    .map_err(Error::Wslc)?,
            );
        }
        Ok(self.session.as_ref().unwrap())
    }

    fn resolve_service_urls(&self, project: &str, environment: &mut Vec<(String, String)>) {
        for (key, value) in environment.iter_mut() {
            for ((proj, svc), addr) in &self.service_addresses {
                if proj != project {
                    continue;
                }
                let from = format!("{svc}:");
                *value = value.replace(&from, &format!("{addr}:"));
            }
        }
    }
}

fn handle_connection(
    stream: TcpStream,
    token: &str,
    state: &mut DaemonState,
) -> Result<()> {
    let mut reader = BufReader::new(&stream);
    let mut request = String::new();
    reader.read_line(&mut request).map_err(|error| {
        Error::InvalidConfig(format!("failed to read SDK daemon request: {error}"))
    })?;
    let request: Request = serde_json::from_str(&request)?;
    if request.token != token {
        let response = Response {
            ok: false,
            payload: serde_json::Value::Null,
            error: Some("token mismatch".to_owned()),
        };
        let mut stream = &stream;
        serde_json::to_writer(&mut stream, &response)?;
        stream.write_all(b"\n").ok();
        return Ok(());
    }
    let response = handle_command(request.command, state);
    let mut stream = &stream;
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n").ok();
    Ok(())
}

fn handle_command(command: CommandRequest, state: &mut DaemonState) -> Response {
    let result: Result<serde_json::Value> = match command {
        CommandRequest::Ping => Ok(serde_json::Value::Bool(true)),
        CommandRequest::Create { project, service } => {
            let cn = service.container_name.clone();
            let session = state.session()?.clone();
            let mut builder = session.container(wslc::ContainerOptions::new(&service.image));
            builder = builder.name(&cn);
            if let Some(hostname) = &service.hostname {
                builder = builder.hostname(hostname);
            }
            if let Some(domain) = &service.domain_name {
                builder = builder.domain_name(domain);
            }
            if service.privileged {
                builder = builder.privileged(true);
            }
            if service.gpus {
                builder = builder.enable_gpu(true);
            }
            for (key, value) in &service.environment {
                builder = builder.env(key, value);
            }
            for port in &service.ports {
                if let Ok((host, container)) = parse_port(port) {
                    builder = builder.port(host, container);
                }
            }
            for mount in &service.mounts {
                builder = builder.volume(&mount.source, &mount.target, mount.read_only);
            }
            if let Some(wd) = &service.working_dir {
                builder = builder.working_dir(wd);
            }
            if !service.command.is_empty() {
                builder = builder.command(&service.command);
            }
            if !service.entrypoint.is_empty() {
                builder = builder.entrypoint(&service.entrypoint);
            }
            let container = builder.build().map_err(Error::Wslc)?;
            state.containers.insert(cn.clone(), container);
            state.projects.insert(cn.clone(), project.clone());
            serde_json::to_value(true).map_err(Error::Json)
        }
        CommandRequest::Exists { name } => {
            serde_json::to_value(state.containers.contains_key(&name)).map_err(Error::Json)
        }
        CommandRequest::Running { name } => {
            let Some(container) = state.containers.get(&name) else {
                return Response {
                    ok: true,
                    payload: serde_json::Value::Bool(false),
                    error: None,
                };
            };
            let state_val = container.state().map_err(Error::Wslc)?;
            serde_json::to_value(matches!(state_val, wslc::ContainerState::Running))
                .map_err(Error::Json)
        }
        CommandRequest::Start { name } => {
            let Some(container) = state.containers.get(&name) else {
                return Response {
                    ok: false,
                    payload: serde_json::Value::Null,
                    error: Some(format!("SDK container not found: {name}")),
                };
            };
            container.start().map_err(Error::Wslc)?;
            serde_json::to_value(true).map_err(Error::Json)
        }
        CommandRequest::Stop { name, signal, timeout } => {
            let Some(container) = state.containers.get(&name) else {
                return Response {
                    ok: false,
                    payload: serde_json::Value::Null,
                    error: Some(format!("SDK container not found: {name}")),
                };
            };
            container
                .stop(parse_signal(&signal), Duration::from_secs(timeout))
                .map_err(Error::Wslc)?;
            serde_json::to_value(true).map_err(Error::Json)
        }
        CommandRequest::Remove { name, force } => {
            let Some(container) = state.containers.swap_remove(&name) else {
                return Response {
                    ok: false,
                    payload: serde_json::Value::Null,
                    error: Some(format!("SDK container not found: {name}")),
                };
            };
            state.projects.remove(&name);
            if force {
                let _ = container.stop(wslc::Signal::Sigkill, Duration::from_secs(0));
            }
            container
                .delete(wslc::DeleteContainerOptions::default().force(force))
                .map_err(Error::Wslc)?;
            serde_json::to_value(true).map_err(Error::Json)
        }
        CommandRequest::Exec {
            name,
            command,
            environment,
            workdir,
        } => {
            let Some(container) = state.containers.get(&name) else {
                return Response {
                    ok: false,
                    payload: serde_json::Value::Null,
                    error: Some(format!("SDK container not found: {name}")),
                };
            };
            let mut process = wslc::ProcessOptions::new(command).capture_stdout();
            if let Some(workdir) = workdir {
                process = process.working_dir(workdir);
            }
            for value in environment {
                let Some((key, value)) = value.split_once('=') else {
                    return Response {
                        ok: false,
                        payload: serde_json::Value::Null,
                        error: Some(format!("invalid exec environment value: {value}")),
                    };
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
    };
    match result {
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
    stream.set_read_timeout(Some(Duration::from_secs(120))).map_err(|error| {
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
}
