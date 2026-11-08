use std::{collections::BTreeMap, process::Stdio};

use serde::Deserialize;

use crate::{
    Error, Result,
    runtime::{
        CONFIG_HASH_LABEL, Container, ContainerRuntime, ContainerState, ExecRequest,
        MANAGED_NETWORK_LABEL, OptionValue, RunRequest,
    },
};

use super::{
    DockerCli,
    options::{append_options, validate_name, validate_options},
    process::exit_code,
};

impl DockerCli {
    fn list(&self, all: bool) -> Result<Vec<Container>> {
        let mut command = self.command(["container", "ls", "--no-trunc", "--format", "{{json .}}"]);
        if all {
            command.arg("--all");
        }
        let output = self.output("list containers", &mut command)?;
        let text = std::str::from_utf8(&output.stdout).map_err(|error| {
            Error::runtime(
                "list containers",
                format!("invalid UTF-8 response: {error}"),
            )
        })?;
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let row: ContainerRow = serde_json::from_str(line).map_err(|error| {
                    Error::runtime("list containers", format!("invalid JSON response: {error}"))
                })?;
                if row.id.is_empty() || row.name.is_empty() {
                    return Err(Error::runtime(
                        "list containers",
                        "missing container ID or name",
                    ));
                }
                Ok(Container {
                    id: row.id,
                    name: row.name,
                    state: parse_state(&row.state),
                    fingerprint: None,
                })
            })
            .collect()
    }
}

impl ContainerRuntime for DockerCli {
    fn validate_run(&self, request: &RunRequest) -> Result<()> {
        validate_name(&request.name)?;
        if request.options.contains_key("cidfile") {
            return Err(Error::Config(
                "run.options.cidfile is reserved for container ownership tracking".into(),
            ));
        }
        if let Some(name) = request.options.get("name") {
            if name != &OptionValue::Text(request.name.clone()) {
                return Err(Error::Config(
                    "run.options.name conflicts with main.name; set main.name instead".into(),
                ));
            }
        }
        validate_options(&request.options)
    }

    fn validate_exec(&self, request: &ExecRequest) -> Result<()> {
        validate_options(&request.options)
    }

    fn inspect(&self, name: &str) -> Result<Option<Container>> {
        let Some(mut container) = self
            .list(true)?
            .into_iter()
            .find(|container| container.name == name)
        else {
            return Ok(None);
        };
        let output = self.output(
            "inspect container labels",
            &mut self.command([
                "container",
                "inspect",
                "--format",
                "{{json .Config.Labels}}",
                "--",
                &container.id,
            ]),
        )?;
        let labels: Option<BTreeMap<String, String>> = serde_json::from_slice(&output.stdout)
            .map_err(|error| {
                Error::runtime(
                    "inspect container labels",
                    format!("invalid JSON response: {error}"),
                )
            })?;
        container.fingerprint = labels.and_then(|mut labels| labels.remove(CONFIG_HASH_LABEL));
        Ok(Some(container))
    }

    fn run(&self, request: &RunRequest) -> Result<Container> {
        self.validate_run(request)?;
        let directory = tempfile::tempdir()
            .map_err(|error| Error::runtime("create container ID file", error.to_string()))?;
        let cidfile = directory.path().join("container-id");
        let mut command = self.command(["run", "--name", &request.name, "--cidfile"]);
        command.arg(&cidfile);
        let mut options = request.options.clone();
        options.remove("name");
        append_options(&mut command, &options)?;
        if let Some(fingerprint) = &request.fingerprint {
            command
                .arg("--label")
                .arg(format!("{CONFIG_HASH_LABEL}={fingerprint}"));
        }
        command
            .arg("--")
            .arg(&request.image)
            .args(&request.command)
            .current_dir(&request.directory)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        self.trace(&command);
        let status = command.status();
        // The ID file ties cleanup to this invocation, even if the name is reused.
        let id = std::fs::read_to_string(&cidfile)
            .ok()
            .map(|id| id.trim().to_owned())
            .filter(|id| id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let cause = match status {
            Ok(status) if status.success() => {
                return id
                    .map(|id| Container {
                        id,
                        name: request.name.clone(),
                        state: ContainerState::Running,
                        fingerprint: request.fingerprint.clone(),
                    })
                    .ok_or_else(|| {
                        Error::runtime(
                            "create container",
                            "Docker did not write a valid container ID",
                        )
                    });
            }
            Ok(status) => Error::runtime(
                "create container",
                format!("exit code {}", exit_code(status)),
            ),
            Err(error) => Error::runtime("create container", error.to_string()),
        };
        if let Some(id) = id {
            if let Err(cleanup) = self.remove(&id, true) {
                return Err(Error::Cleanup {
                    cause: Box::new(cause),
                    cleanup: Box::new(cleanup),
                });
            }
        }
        Err(cause)
    }

    fn start(&self, id: &str) -> Result<()> {
        self.action("start container", &["start", "--", id])
    }

    fn exec(&self, id: &str, request: &ExecRequest) -> Result<i32> {
        let mut command = self.command(["exec"]);
        append_options(&mut command, &request.options)?;
        for variable in &request.env {
            if !variable.as_encoded_bytes().contains(&b'=') {
                // An unset value in the snapshot must not inherit a later host value.
                command.env_remove(variable);
            }
            command.arg("--env").arg(variable);
        }
        command
            .arg("--")
            .arg(id)
            .args(&request.command)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        self.trace(&command);
        let status = command
            .status()
            .map_err(|error| Error::runtime("execute command", error.to_string()))?;
        Ok(exit_code(status))
    }

    fn stop(&self, id: &str) -> Result<()> {
        self.action("stop container", &["stop", "--", id])
    }

    fn remove(&self, id: &str, force: bool) -> Result<()> {
        if force {
            self.action("remove container", &["rm", "--force", "--", id])
        } else {
            self.action("remove container", &["rm", "--", id])
        }
    }

    fn list_running(&self) -> Result<Vec<Container>> {
        self.list(false)
    }

    fn create_networks(&self, names: &[String]) {
        self.network_action(
            &[
                "create",
                "--label",
                &format!("{MANAGED_NETWORK_LABEL}=true"),
            ],
            names,
        );
    }

    fn remove_networks(&self, names: &[String]) {
        self.network_action(&["rm"], names);
    }
}

#[derive(Deserialize)]
struct ContainerRow {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Names")]
    name: String,
    #[serde(rename = "State")]
    state: String,
}

fn parse_state(state: &str) -> ContainerState {
    match state {
        "created" => ContainerState::Created,
        "running" => ContainerState::Running,
        "exited" => ContainerState::Exited,
        "paused" => ContainerState::Paused,
        "restarting" => ContainerState::Restarting,
        "removing" => ContainerState::Removing,
        "dead" => ContainerState::Dead,
        other => ContainerState::Unknown(other.into()),
    }
}
