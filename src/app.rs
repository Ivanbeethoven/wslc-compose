use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::{ExecOptions, LogOptions, OneOffOptions, WslcBackend};
use crate::cli::{Command, OutputFormat, PsFormat, PullPolicy};
use crate::config::{self, LoadOptions};
use crate::model::{Project, Service};
use crate::plan::service_order;
use crate::{Cli, Error, Result};

pub fn run(cli: Cli) -> Result<()> {
    if let Command::Version { short } = cli.command {
        return version(short);
    }

    let loaded = config::load(LoadOptions {
        files: cli.files,
        project_name: cli.project_name,
        project_directory: cli.project_directory,
        env_file: cli.env_file,
    })?;
    let project = loaded.project;
    let profiles = cli.profile;

    match cli.command {
        Command::Config {
            quiet,
            services,
            profiles: list_profiles,
            format,
        } => {
            if quiet {
                return Ok(());
            }
            if services {
                for service in service_order(&project, &[], &profiles, true)? {
                    println!("{service}");
                }
            } else if list_profiles {
                let values: BTreeSet<_> = project
                    .services
                    .values()
                    .flat_map(|service| service.profiles.iter())
                    .collect();
                for profile in values {
                    println!("{profile}");
                }
            } else {
                match format {
                    OutputFormat::Yaml => print!("{}", serde_yaml::to_string(&loaded.rendered)?),
                    OutputFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&loaded.rendered)?)
                    }
                }
            }
            Ok(())
        }
        Command::Pull {
            services,
            ignore_pull_failures,
            quiet: _,
        } => {
            let backend = runtime(&project)?;
            for name in service_order(&project, &services, &profiles, true)? {
                let service = &project.services[&name];
                if service
                    .build
                    .as_ref()
                    .is_some_and(|build| build.generated_tag)
                {
                    eprintln!("warning: {name} has a locally generated image; skipping pull");
                    continue;
                }
                let image = require_image(service)?;
                println!("[+] Pulling {name} ({image})");
                if let Err(error) = backend.pull(image) {
                    if ignore_pull_failures {
                        eprintln!("warning: {error}");
                    } else {
                        return Err(error);
                    }
                }
            }
            Ok(())
        }
        Command::Build {
            services,
            no_cache,
            pull,
            quiet: _,
        } => {
            let backend = runtime(&project)?;
            for name in service_order(&project, &services, &profiles, true)? {
                let service = &project.services[&name];
                if let Some(build) = &service.build {
                    println!("[+] Building {name} ({})", build.tag);
                    backend.build(build, no_cache, pull)?;
                } else if !services.is_empty() {
                    eprintln!("warning: service {name} has no build configuration");
                }
            }
            Ok(())
        }
        Command::Create {
            services,
            build,
            force_recreate,
            no_recreate,
            pull,
        } => {
            let backend = runtime(&project)?;
            let order = service_order(&project, &services, &profiles, true)?;
            create_services(
                &backend,
                &project,
                &order,
                force_recreate,
                no_recreate,
                pull,
                if build {
                    BuildPolicy::Force
                } else {
                    BuildPolicy::Implicit
                },
            )
        }
        Command::Up {
            services,
            detach,
            no_start,
            build,
            no_build,
            force_recreate,
            no_recreate,
            pull,
            remove_orphans,
        } => {
            let backend = runtime(&project)?;
            let order = service_order(&project, &services, &profiles, true)?;
            if remove_orphans {
                eprintln!("warning: --remove-orphans is not implemented yet");
            }
            create_services(
                &backend,
                &project,
                &order,
                force_recreate,
                no_recreate,
                pull,
                if no_build {
                    BuildPolicy::Never
                } else if build {
                    BuildPolicy::Force
                } else {
                    BuildPolicy::Implicit
                },
            )?;
            if !no_start {
                for name in &order {
                    let service = &project.services[name];
                    if service.privileged {
                        println!("[=] Container {name} started via SDK (privileged)");
                        continue;
                    }
                    if backend.project_container_running(&project, &service.container_name)? {
                        println!("[=] Container {name} is already running");
                    } else {
                        println!("[+] Starting {name}");
                        backend.start(&service.container_name)?;
                    }
                }
            }
            if !detach && !no_start {
                if order.len() == 1 {
                    let service = &project.services[&order[0]];
                    backend.logs(
                        &service.container_name,
                        &LogOptions {
                            follow: true,
                            tail: None,
                            timestamps: false,
                            since: None,
                            until: None,
                        },
                    )?;
                } else {
                    eprintln!(
                        "warning: multi-service attached log multiplexing is not implemented; services are running detached"
                    );
                }
            }
            Ok(())
        }
        Command::Down {
            volumes,
            timeout,
            remove_orphans,
        } => {
            let backend = runtime(&project)?;
            if remove_orphans {
                eprintln!("warning: --remove-orphans is not implemented yet");
            }
            let mut order = service_order(&project, &[], &profiles, true)?;
            order.reverse();
            for name in order {
                let service = &project.services[&name];
                if backend.project_container_exists(&project, &service.container_name)? {
                    println!("[+] Stopping {name}");
                    if let Err(error) =
                        backend.stop(&service.container_name, &service.stop_signal, timeout)
                    {
                        eprintln!("warning: {error}");
                    }
                    println!("[+] Removing {name}");
                    backend.remove(&service.container_name, true)?;
                }
            }
            backend.remove_project_resources(&project, volumes)
        }
        Command::Start { services } => {
            let backend = runtime(&project)?;
            for name in service_order(&project, &services, &profiles, true)? {
                let service = &project.services[&name];
                println!("[+] Starting {name}");
                if !backend.project_container_running(&project, &service.container_name)? {
                    backend.start(&service.container_name)?;
                }
            }
            Ok(())
        }
        Command::Stop { services, timeout } => {
            let backend = runtime(&project)?;
            let mut order = service_order(&project, &services, &profiles, false)?;
            order.reverse();
            for name in order {
                let service = &project.services[&name];
                println!("[+] Stopping {name}");
                backend.stop(&service.container_name, &service.stop_signal, timeout)?;
            }
            Ok(())
        }
        Command::Restart { services, timeout } => {
            let backend = runtime(&project)?;
            let order = service_order(&project, &services, &profiles, true)?;
            for name in order.iter().rev() {
                let service = &project.services[name];
                println!("[+] Stopping {name}");
                backend.stop(&service.container_name, &service.stop_signal, timeout)?;
            }
            for name in order {
                println!("[+] Starting {name}");
                backend.start(&project.services[&name].container_name)?;
            }
            Ok(())
        }
        Command::Ps {
            services,
            all,
            quiet,
            format,
        } => {
            let backend = runtime(&project)?;
            let _ = service_order(&project, &services, &profiles, false)?;
            backend.ps(&project, all, quiet, format == PsFormat::Json)
        }
        Command::Logs {
            services,
            follow,
            tail,
            timestamps,
            since,
            until,
        } => {
            let backend = runtime(&project)?;
            let order = service_order(&project, &services, &profiles, false)?;
            if follow && order.len() > 1 {
                return Err(Error::InvalidConfig(
                    "--follow currently supports exactly one service".to_owned(),
                ));
            }
            let options = LogOptions {
                follow,
                tail,
                timestamps,
                since: since.as_deref(),
                until: until.as_deref(),
            };
            for name in order {
                if !follow {
                    println!("==> {name} <==");
                }
                backend.logs(&project.services[&name].container_name, &options)?;
            }
            Ok(())
        }
        Command::Exec {
            detach,
            interactive,
            tty,
            environment,
            user,
            workdir,
            service,
            command,
        } => {
            let backend = runtime(&project)?;
            let service = project
                .services
                .get(&service)
                .ok_or_else(|| Error::UnknownService(service.clone()))?;
            backend.exec(
                &service.container_name,
                ExecOptions {
                    command: &command,
                    environment: &environment,
                    detach,
                    interactive,
                    tty,
                    user: user.as_deref(),
                    workdir: workdir.as_deref(),
                },
            )
        }
        Command::Run {
            detach,
            rm,
            no_deps,
            environment,
            name,
            service_ports,
            service,
            command,
        } => {
            let backend = runtime(&project)?;
            let target = project
                .services
                .get(&service)
                .ok_or_else(|| Error::UnknownService(service.clone()))?;
            backend.ensure_project_resources(&project)?;
            if !no_deps {
                let dependencies =
                    service_order(&project, std::slice::from_ref(&service), &profiles, true)?;
                for dependency in dependencies.into_iter().filter(|name| name != &service) {
                    let dependency = &project.services[&dependency];
                    if !backend.project_container_exists(&project, &dependency.container_name)? {
                        backend.create(&project, dependency)?;
                    }
                    if let Err(error) = backend.start(&dependency.container_name) {
                        eprintln!("warning: {error}");
                    }
                }
            }
            let generated_name = name.unwrap_or_else(|| {
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                format!("{}-{}-run-{timestamp}", project.name, target.name)
            });
            backend.run_one_off(
                &project,
                target,
                OneOffOptions {
                    name: &generated_name,
                    command: &command,
                    environment: &environment,
                    detach,
                    remove: rm,
                    service_ports,
                },
            )
        }
        Command::Kill { services, signal } => {
            let backend = runtime(&project)?;
            for name in service_order(&project, &services, &profiles, false)? {
                backend.kill(&project.services[&name].container_name, &signal)?;
            }
            Ok(())
        }
        Command::Rm {
            services,
            force,
            stop,
            volumes,
        } => {
            let backend = runtime(&project)?;
            let mut order = service_order(&project, &services, &profiles, false)?;
            order.reverse();
            for name in order {
                let service = &project.services[&name];
                if stop {
                    let _ = backend.stop(
                        &service.container_name,
                        &service.stop_signal,
                        service.stop_grace_period.as_secs(),
                    );
                }
                backend.remove(&service.container_name, force)?;
            }
            if volumes {
                for resource in project.volumes.values() {
                    if !resource.external {
                        eprintln!(
                            "warning: named volume {} is retained by `rm`; use `down --volumes`",
                            resource.name
                        );
                    }
                }
            }
            Ok(())
        }
        Command::Version { .. } => unreachable!("handled before loading project"),
    }
}

fn runtime(project: &Project) -> Result<WslcBackend> {
    let backend = WslcBackend::new(&project.working_dir);
    let _ = backend.ensure_available()?;
    Ok(backend)
}

#[derive(Clone, Copy)]
enum BuildPolicy {
    Force,
    Implicit,
    Never,
}

fn create_services(
    backend: &WslcBackend,
    project: &Project,
    order: &[String],
    force_recreate: bool,
    no_recreate: bool,
    pull: PullPolicy,
    build_policy: BuildPolicy,
) -> Result<()> {
    let availability = backend.ensure_available()?;
    let sdk_project = backend.uses_sdk_project(project);
    if !sdk_project {
        backend.ensure_project_resources(project)?;
    } else {
        for name in order {
            validate_sdk_compatibility(&project.services[name])?;
        }
    }
    for name in order {
        let service = &project.services[name];
        warn_compatibility(service, availability.sdk_version.is_some())?;
        let image = require_image(service)?;
        let built = if let Some(build) = &service.build {
            if sdk_project {
                return Err(Error::Unsupported {
                    service: service.name.clone(),
                    feature: "build is not available for SDK-backed Compose projects; publish or pull an image instead".to_owned(),
                });
            }
            if should_build(build_policy, build.generated_tag) {
                println!("[+] Building {name} ({})", build.tag);
                backend.build(build, false, pull == PullPolicy::Always)?;
                true
            } else {
                false
            }
        } else {
            false
        };
        if !sdk_project && !built && pull != PullPolicy::Never {
            println!("[+] Pulling {name} ({image})");
            backend.pull(image)?;
        }

        let exists = backend.project_container_exists(project, &service.container_name)?;
        if exists && force_recreate {
            println!("[+] Recreating {name}");
            backend.remove(&service.container_name, true)?;
        } else if exists && (no_recreate || !force_recreate) {
            println!("[=] Container {name} already exists");
            continue;
        }
        println!("[+] Creating {name}");
        backend.create(project, service)?;
    }
    Ok(())
}

fn should_build(policy: BuildPolicy, generated_tag: bool) -> bool {
    match policy {
        BuildPolicy::Force => true,
        BuildPolicy::Implicit => generated_tag,
        BuildPolicy::Never => false,
    }
}

fn require_image(service: &Service) -> Result<&str> {
    service.image.as_deref().ok_or_else(|| Error::MissingImage {
        service: service.name.clone(),
    })
}

fn warn_compatibility(service: &Service, sdk_available: bool) -> Result<()> {
    if service.privileged && !sdk_available {
        return Err(Error::Unsupported {
            service: service.name.clone(),
            feature: "privileged requires WSLC SDK (wslcsdk.dll not visible)".to_owned(),
        });
    }
    if service.restart.is_some() {
        eprintln!(
            "warning: service {} restart policy is not enforced yet",
            service.name
        );
    }
    if !service.unsupported.is_empty() {
        eprintln!(
            "warning: service {} fields are validated but not applied by WSLC yet: {}",
            service.name,
            service.unsupported.join(", ")
        );
    }
    Ok(())
}

fn validate_sdk_compatibility(service: &Service) -> Result<()> {
    if !service.ulimits.is_empty() {
        return Err(Error::Unsupported {
            service: service.name.clone(),
            feature: "ulimits are not exposed by the WSLC SDK backend".to_owned(),
        });
    }
    let unsupported_runtime = ["devices", "cap_add", "security_opt"];
    if let Some(feature) = service
        .unsupported
        .iter()
        .find(|feature| unsupported_runtime.contains(&feature.as_str()))
    {
        return Err(Error::Unsupported {
            service: service.name.clone(),
            feature: format!("{feature} is not exposed by the WSLC SDK backend"),
        });
    }
    Ok(())
}

fn version(short: bool) -> Result<()> {
    if short {
        println!(env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    println!("wslc-compose {}", env!("CARGO_PKG_VERSION"));
    let backend = WslcBackend::new(std::env::current_dir().unwrap_or_default());
    let availability = backend.ensure_available()?;
    println!("{}", availability.cli_version);
    if let Some(version) = availability.sdk_version {
        println!(
            "WSLC SDK {}.{}.{}",
            version.major, version.minor, version.revision
        );
    } else {
        println!("WSLC SDK: DLL not visible to wslc-rs; using wslc.exe backend");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{should_build, BuildPolicy};

    #[test]
    fn no_build_disables_forced_and_implicit_builds() {
        assert!(!should_build(BuildPolicy::Never, true));
        assert!(!should_build(BuildPolicy::Never, false));
        assert!(should_build(BuildPolicy::Force, false));
        assert!(should_build(BuildPolicy::Implicit, true));
        assert!(!should_build(BuildPolicy::Implicit, false));
    }
}
