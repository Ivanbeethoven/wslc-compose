use std::path::{Path, PathBuf};

use serde_yaml::Value;

use crate::env::{self, Environment};
use crate::model::{NormalizeOptions, Project, RawCompose};
use crate::{Error, Result};

const COMPOSE_FILENAMES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];

#[derive(Debug)]
pub struct LoadOptions {
    pub files: Vec<PathBuf>,
    pub project_name: Option<String>,
    pub project_directory: Option<PathBuf>,
    pub env_file: Option<PathBuf>,
}

#[derive(Debug)]
pub struct LoadedProject {
    pub project: Project,
    pub rendered: Value,
}

pub fn load(options: LoadOptions) -> Result<LoadedProject> {
    let current_dir = std::env::current_dir().map_err(|source| Error::ReadFile {
        path: PathBuf::from("."),
        source,
    })?;
    let discovery_dir = options
        .project_directory
        .as_deref()
        .map(|path| resolve_path(&current_dir, path))
        .unwrap_or_else(|| current_dir.clone());
    let files = resolve_files(&options.files, &current_dir, &discovery_dir)?;
    let working_dir = options
        .project_directory
        .as_deref()
        .map(|path| resolve_path(&current_dir, path))
        .unwrap_or_else(|| {
            files[0]
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| current_dir.clone())
        });

    let env_path = options
        .env_file
        .as_deref()
        .map(|path| resolve_path(&current_dir, path))
        .unwrap_or_else(|| working_dir.join(".env"));
    let environment = env::collect(Some(&env_path))?;

    let mut rendered = Value::Mapping(Default::default());
    for file in &files {
        let source = std::fs::read_to_string(file).map_err(|source| Error::ReadFile {
            path: file.clone(),
            source,
        })?;
        let mut value: Value =
            serde_yaml::from_str(&source).map_err(|source| Error::ParseYaml {
                path: file.clone(),
                source,
            })?;
        interpolate_value(&mut value, &environment)?;
        merge(&mut rendered, value);
    }

    let serialized = serde_yaml::to_string(&rendered)?;
    let validation_value = compose_spec_validation_value(&rendered);
    let validation_yaml = serde_yaml::to_string(&validation_value)?;
    let _: compose_spec::Compose =
        serde_yaml::from_str(&validation_yaml).map_err(Error::ComposeValidation)?;
    let raw: RawCompose = serde_yaml::from_str(&serialized)?;

    let project_name = project_name(
        options.project_name.as_deref(),
        raw.name.as_deref(),
        &working_dir,
    )?;
    let project = Project::normalize(
        raw,
        NormalizeOptions {
            project_name: &project_name,
            working_dir: &working_dir,
            source_files: files,
            host_env: &environment,
        },
    )?;

    Ok(LoadedProject { project, rendered })
}

fn compose_spec_validation_value(rendered: &Value) -> Value {
    let mut value = rendered.clone();
    if !cfg!(windows) {
        return value;
    }

    let Some(services) = value
        .as_mapping_mut()
        .and_then(|root| root.get_mut(Value::String("services".to_owned())))
        .and_then(Value::as_mapping_mut)
    else {
        return value;
    };
    for service in services.values_mut().filter_map(Value::as_mapping_mut) {
        // compose_spec 0.3 uses host Path::is_absolute for Linux container paths.
        for key in ["volumes", "working_dir", "configs", "secrets"] {
            service.remove(Value::String(key.to_owned()));
        }
    }
    value
}

fn resolve_files(
    explicit: &[PathBuf],
    current_dir: &Path,
    discovery_dir: &Path,
) -> Result<Vec<PathBuf>> {
    if !explicit.is_empty() {
        return explicit
            .iter()
            .map(|path| existing_file(resolve_path(current_dir, path)))
            .collect();
    }

    if let Ok(compose_file) = std::env::var("COMPOSE_FILE") {
        let separator = std::env::var("COMPOSE_PATH_SEPARATOR").unwrap_or_else(|_| ";".to_owned());
        let files = compose_file
            .split(&separator)
            .filter(|value| !value.is_empty())
            .map(|value| existing_file(resolve_path(current_dir, Path::new(value))))
            .collect::<Result<Vec<_>>>()?;
        if !files.is_empty() {
            return Ok(files);
        }
    }

    COMPOSE_FILENAMES
        .iter()
        .map(|name| discovery_dir.join(name))
        .find(|path| path.is_file())
        .map(|path| vec![path])
        .ok_or(Error::ComposeFileNotFound)
}

fn existing_file(path: PathBuf) -> Result<PathBuf> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(Error::ReadFile {
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "file does not exist"),
            path,
        })
    }
}

fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn project_name(cli: Option<&str>, top_level: Option<&str>, working_dir: &Path) -> Result<String> {
    let candidate = cli
        .or(top_level)
        .map(str::to_owned)
        .or_else(|| {
            working_dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .ok_or_else(|| Error::InvalidConfig("could not determine project name".to_owned()))?;

    let normalized: String = candidate
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .skip_while(|ch| !ch.is_ascii_alphanumeric())
        .collect();
    if normalized.is_empty() {
        return Err(Error::InvalidConfig(format!(
            "project name contains no valid characters: {candidate}"
        )));
    }
    Ok(normalized)
}

fn interpolate_value(value: &mut Value, environment: &Environment) -> Result<()> {
    match value {
        Value::String(string) => *string = env::interpolate(string, environment)?,
        Value::Sequence(values) => {
            for value in values {
                interpolate_value(value, environment)?;
            }
        }
        Value::Mapping(mapping) => {
            let old = std::mem::take(mapping);
            for (mut key, mut value) in old {
                interpolate_value(&mut key, environment)?;
                interpolate_value(&mut value, environment)?;
                mapping.insert(key, value);
            }
        }
        Value::Tagged(tagged) => interpolate_value(&mut tagged.value, environment)?,
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

pub fn merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Mapping(base), Value::Mapping(overlay)) => {
            for (key, value) in overlay {
                if let Some(existing) = base.get_mut(&key) {
                    merge(existing, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (Value::Sequence(base), Value::Sequence(mut overlay)) => base.append(&mut overlay),
        (base, overlay) => *base = overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_mapping_recursively_and_appends_lists() {
        let mut base: Value =
            serde_yaml::from_str("services:\n  web:\n    image: old\n    ports: [\"80:80\"]\n")
                .unwrap();
        let overlay: Value =
            serde_yaml::from_str("services:\n  web:\n    image: new\n    ports: [\"443:443\"]\n")
                .unwrap();
        merge(&mut base, overlay);
        let text = serde_yaml::to_string(&base).unwrap();
        assert!(text.contains("image: new"));
        assert!(text.contains("80:80"));
        assert!(text.contains("443:443"));
    }

    #[test]
    fn loads_and_interpolates_a_compose_project() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("compose.yaml");
        std::fs::write(
            &file,
            "name: demo-app\nservices:\n  web:\n    image: alpine:${TAG:-latest}\n",
        )
        .unwrap();
        let loaded = load(LoadOptions {
            files: vec![file],
            project_name: None,
            project_directory: Some(temp.path().to_path_buf()),
            env_file: None,
        })
        .unwrap();
        assert_eq!(loaded.project.name, "demo-app");
        assert_eq!(
            loaded.project.services["web"].image.as_deref(),
            Some("alpine:latest")
        );
    }

    #[test]
    fn build_only_service_gets_a_stable_project_image() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("compose.yaml");
        std::fs::write(
            &file,
            "name: demo\nservices:\n  api:\n    build:\n      context: .\n      target: runtime\n",
        )
        .unwrap();
        let loaded = load(LoadOptions {
            files: vec![file],
            project_name: None,
            project_directory: Some(temp.path().to_path_buf()),
            env_file: None,
        })
        .unwrap();
        let service = &loaded.project.services["api"];
        assert_eq!(service.image.as_deref(), Some("demo-api:latest"));
        assert!(service.build.as_ref().unwrap().generated_tag);
        assert_eq!(
            service.build.as_ref().unwrap().target.as_deref(),
            Some("runtime")
        );
    }
}
