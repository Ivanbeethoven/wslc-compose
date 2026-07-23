use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Compose file found; expected compose.yaml, compose.yml, docker-compose.yaml, or docker-compose.yml")]
    ComposeFileNotFound,

    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse {path}: {source}")]
    ParseYaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("Compose validation failed: {0}")]
    ComposeValidation(serde_yaml::Error),

    #[error("invalid Compose configuration: {0}")]
    InvalidConfig(String),

    #[error("environment variable {name} is required: {message}")]
    RequiredVariable { name: String, message: String },

    #[error("service dependency cycle: {0}")]
    DependencyCycle(String),

    #[error("unknown service: {0}")]
    UnknownService(String),

    #[error("service {service} requires an image; build-only services are not supported yet")]
    MissingImage { service: String },

    #[error("unsupported Compose feature for service {service}: {feature}")]
    Unsupported { service: String, feature: String },

    #[error("failed to start wslc.exe: {0}")]
    StartWslc(#[source] std::io::Error),

    #[error("wslc {command} failed with exit code {code}: {message}")]
    WslcCommand {
        command: String,
        code: i32,
        message: String,
    },

    #[error(transparent)]
    Wslc(#[from] wslc::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
