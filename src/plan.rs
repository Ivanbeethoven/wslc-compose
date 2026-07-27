use std::collections::{HashMap, HashSet};

use crate::model::Project;
use crate::{Error, Result};

pub fn service_order(
    project: &Project,
    requested: &[String],
    profiles: &[String],
    include_dependencies: bool,
) -> Result<Vec<String>> {
    let enabled_profiles: HashSet<&str> = profiles.iter().map(String::as_str).collect();
    let mut selected = HashSet::new();

    if requested.is_empty() {
        for (name, service) in &project.services {
            if service.profiles.is_empty()
                || service
                    .profiles
                    .iter()
                    .any(|profile| enabled_profiles.contains(profile.as_str()))
            {
                selected.insert(name.clone());
            }
        }
    } else {
        for name in requested {
            if !project.services.contains_key(name) {
                return Err(Error::UnknownService(name.clone()));
            }
            select(name, project, include_dependencies, &mut selected)?;
        }
    }

    topological_order(project, &selected)
}

fn select(
    name: &str,
    project: &Project,
    dependencies: bool,
    selected: &mut HashSet<String>,
) -> Result<()> {
    if !selected.insert(name.to_owned()) {
        return Ok(());
    }
    if dependencies {
        let service = project
            .services
            .get(name)
            .ok_or_else(|| Error::UnknownService(name.to_owned()))?;
        for dependency in &service.depends_on {
            select(dependency, project, true, selected)?;
        }
    }
    Ok(())
}

fn topological_order(project: &Project, selected: &HashSet<String>) -> Result<Vec<String>> {
    let mut indegree: HashMap<&str, usize> = selected
        .iter()
        .map(|name| (name.as_str(), 0usize))
        .collect();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for name in selected {
        let service = &project.services[name];
        for dependency in &service.depends_on {
            if selected.contains(dependency) {
                *indegree.get_mut(name.as_str()).expect("selected service") += 1;
                dependents
                    .entry(dependency.as_str())
                    .or_default()
                    .push(name.as_str());
            }
        }
    }

    let mut order: Vec<String> = Vec::with_capacity(selected.len());
    loop {
        let next = project
            .services
            .keys()
            .find(|name| {
                selected.contains(name.as_str())
                    && !order.iter().any(|item| item == *name)
                    && indegree.get(name.as_str()) == Some(&0)
            })
            .cloned();
        let Some(next) = next else { break };
        order.push(next.clone());
        if let Some(values) = dependents.get(next.as_str()) {
            for dependent in values {
                *indegree.get_mut(dependent).expect("dependent service") -= 1;
            }
        }
    }

    if order.len() != selected.len() {
        let cycle = selected
            .iter()
            .filter(|name| !order.contains(name))
            .cloned()
            .collect::<Vec<_>>()
            .join(" -> ");
        return Err(Error::DependencyCycle(cycle));
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;
    use crate::model::{Project, Service};

    fn service(name: &str, dependencies: &[&str], profiles: &[&str]) -> Service {
        Service {
            name: name.to_owned(),
            image: Some("alpine:latest".to_owned()),
            build: None,
            container_name: format!("demo-{name}-1"),
            command: Vec::new(),
            entrypoint: Vec::new(),
            environment: IndexMap::new(),
            ports: Vec::new(),
            mounts: Vec::new(),
            depends_on: dependencies
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            profiles: profiles.iter().map(|value| (*value).to_owned()).collect(),
            labels: IndexMap::new(),
            networks: Vec::new(),
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
            stop_signal: "SIGTERM".to_owned(),
            stop_grace_period: std::time::Duration::from_secs(10),
            restart: None,
            unsupported: Vec::new(),
        }
    }

    fn project() -> Project {
        Project {
            name: "demo".to_owned(),
            working_dir: ".".into(),
            source_files: Vec::new(),
            services: IndexMap::from([
                ("db".to_owned(), service("db", &[], &[])),
                ("api".to_owned(), service("api", &["db"], &[])),
                ("debug".to_owned(), service("debug", &["api"], &["tools"])),
            ]),
            networks: IndexMap::new(),
            volumes: IndexMap::new(),
        }
    }

    #[test]
    fn orders_dependencies_before_dependents() {
        assert_eq!(
            service_order(&project(), &["api".to_owned()], &[], true).unwrap(),
            ["db", "api"]
        );
    }

    #[test]
    fn profiles_control_implicit_selection() {
        assert_eq!(
            service_order(&project(), &[], &[], true).unwrap(),
            ["db", "api"]
        );
        assert_eq!(
            service_order(&project(), &[], &["tools".to_owned()], true).unwrap(),
            ["db", "api", "debug"]
        );
    }

    #[test]
    fn reports_dependency_cycles() {
        let mut project = project();
        project.services["db"].depends_on.push("api".to_owned());
        let error = service_order(&project, &[], &[], true).unwrap_err();
        assert!(matches!(error, Error::DependencyCycle(_)));
    }
}
