use std::ffi::OsString;

use crate::{
    Error, Result,
    config::Config,
    runtime::{Container, ContainerRuntime, ContainerState},
};

pub fn execute(
    runtime: &(impl ContainerRuntime + ?Sized),
    config: &Config,
    command: &[OsString],
    confirm_recreation: &mut dyn FnMut() -> Result<bool>,
) -> Result<i32> {
    let request = config.exec_request(command)?;
    runtime.validate_exec(&request)?;
    let mut existing = runtime.inspect(&config.main.name)?;
    let recreate = match &existing {
        Some(container)
            if config.run.check_zenvfile_changes
                && container.fingerprint.as_deref() != Some(config.fingerprint.as_str()) =>
        {
            confirm_recreation()?
        }
        _ => false,
    };
    if existing.is_none() || recreate {
        runtime.validate_run(&config.run_request())?;
        for command in &config.run.init_commands {
            runtime.validate_exec(command)?;
        }
    }
    if recreate {
        let container = existing
            .take()
            .expect("recreation requires an existing container");
        remove_container(runtime, &container)?;
        remove_networks(runtime, config);
    }
    let container = match existing {
        None => {
            if config.run.manage_network && !config.run.networks.is_empty() {
                runtime.create_networks(&config.run.networks);
            }
            let container = runtime.run(&config.run_request())?;
            for command in &config.run.init_commands {
                let result = runtime.exec(&container.id, command).and_then(|code| {
                    if code == 0 {
                        Ok(())
                    } else {
                        Err(Error::runtime(
                            "init",
                            format!("command {:?} exited with {code}", command.command),
                        ))
                    }
                });
                if let Err(cause) = result {
                    return match runtime.remove(&container.id, true) {
                        Ok(()) => Err(cause),
                        Err(cleanup) => Err(Error::Cleanup {
                            cause: Box::new(cause),
                            cleanup: Box::new(cleanup),
                        }),
                    };
                }
            }
            container
        }
        Some(container) => {
            match container.state {
                ContainerState::Created | ContainerState::Exited => runtime.start(&container.id)?,
                ContainerState::Running => {}
                _ => {
                    return Err(Error::runtime(
                        "exec",
                        format!("container {} is {}", container.name, container.state),
                    ));
                }
            }
            container
        }
    };
    runtime.exec(&container.id, &request)
}

pub fn stop(runtime: &(impl ContainerRuntime + ?Sized), name: &str) -> Result<()> {
    if let Some(container) = runtime.inspect(name)? {
        runtime.stop(&container.id)?;
    }
    Ok(())
}

pub fn remove(runtime: &(impl ContainerRuntime + ?Sized), config: &Config) -> Result<()> {
    if let Some(container) = runtime.inspect(&config.main.name)? {
        remove_container(runtime, &container)?;
    }
    remove_networks(runtime, config);
    Ok(())
}

fn remove_container(
    runtime: &(impl ContainerRuntime + ?Sized),
    container: &Container,
) -> Result<()> {
    match container.state {
        ContainerState::Running => {
            runtime.stop(&container.id)?;
            runtime.remove(&container.id, false)
        }
        ContainerState::Paused | ContainerState::Restarting => runtime.remove(&container.id, true),
        _ => runtime.remove(&container.id, false),
    }
}

fn remove_networks(runtime: &(impl ContainerRuntime + ?Sized), config: &Config) {
    if config.run.manage_network && !config.run.networks.is_empty() {
        runtime.remove_networks(&config.run.networks);
    }
}

pub fn stop_all(runtime: &(impl ContainerRuntime + ?Sized), exclude: Option<&str>) -> Result<()> {
    let mut errors = Vec::new();
    for container in runtime.list_running()? {
        if container.name.starts_with("zenv-") && exclude != Some(container.name.as_str()) {
            if let Err(error) = runtime.stop(&container.id) {
                errors.push(format!("{}: {error}", container.name));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(Error::Multiple(errors.join("\n")))
    }
}
