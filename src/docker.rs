//! Docker CLI adapter. Docker syntax and response formats stay in this module.

use std::{
    ffi::{OsStr, OsString},
    io::{self, Write},
    os::unix::process::ExitStatusExt,
    process::{Command, ExitStatus, Output, Stdio},
};

use serde::Deserialize;

use crate::{
    Error, Result,
    runtime::{
        Container, ContainerRuntime, ContainerState, ExecRequest, OptionValue, Options, RunRequest,
    },
};

#[derive(Clone, Debug)]
pub struct DockerCli {
    program: OsString,
    debug: bool,
}

impl DockerCli {
    pub fn new(debug: bool) -> Self {
        Self::with_program("docker", debug)
    }

    /// Select an executable, for example a Docker CLI installed outside PATH.
    pub fn with_program(program: impl Into<OsString>, debug: bool) -> Self {
        Self {
            program: program.into(),
            debug,
        }
    }

    fn command(&self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Command {
        let mut command = Command::new(&self.program);
        command.args(args);
        command
    }

    fn trace(&self, command: &Command) {
        if self.debug {
            // Environment values and user arguments may contain secrets.
            eprintln!(
                "Docker: {:?} {:?}",
                command.get_program(),
                command.get_args().next()
            );
        }
    }

    fn output(&self, operation: &str, command: &mut Command) -> Result<Output> {
        self.trace(command);
        let output = command
            .output()
            .map_err(|error| Error::runtime(operation, error.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::runtime(
                operation,
                format!("exit code {}: {}", exit_code(output.status), stderr.trim()),
            ));
        }
        Ok(output)
    }

    fn action(&self, operation: &str, args: &[&str]) -> Result<()> {
        let output = self.output(operation, &mut self.command(args))?;
        io::stdout()
            .write_all(&output.stdout)
            .map_err(|error| Error::runtime(operation, error.to_string()))?;
        io::stderr()
            .write_all(&output.stderr)
            .map_err(|error| Error::runtime(operation, error.to_string()))?;
        Ok(())
    }

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
                })
            })
            .collect()
    }
}

impl ContainerRuntime for DockerCli {
    fn inspect(&self, name: &str) -> Result<Option<Container>> {
        Ok(self
            .list(true)?
            .into_iter()
            .find(|container| container.name == name))
    }

    fn run(&self, request: &RunRequest) -> Result<Container> {
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
        let directory = tempfile::tempdir()
            .map_err(|error| Error::runtime("create container ID file", error.to_string()))?;
        let cidfile = directory.path().join("container-id");
        let mut command = self.command(["run", "--name", &request.name, "--cidfile"]);
        command.arg(&cidfile);
        let mut options = request.options.clone();
        options.remove("name");
        append_options(&mut command, &options)?;
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
        command.arg("--").arg(id).args(&request.command);
        command
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

fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.starts_with('-') {
        return Err(Error::Config(
            "container name must be nonempty and must not start with '-'".into(),
        ));
    }
    Ok(())
}

fn append_options(command: &mut Command, options: &Options) -> Result<()> {
    for (name, value) in options {
        if name.is_empty()
            || name.starts_with('-')
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(Error::Config(format!(
                "invalid runtime option name: {name:?}"
            )));
        }
        let option = format!("--{name}");
        match value {
            OptionValue::Flag(true) => {
                command.arg(option);
            }
            OptionValue::Flag(false) => {}
            OptionValue::Text(value) => {
                command.arg(option).arg(value);
            }
            OptionValue::Integer(value) => {
                command.arg(option).arg(value.to_string());
            }
            OptionValue::List(values) => {
                for value in values {
                    command.arg(&option).arg(value);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::BTreeMap,
        fs,
        os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    };

    const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    struct FakeDocker {
        directory: tempfile::TempDir,
        adapter: DockerCli,
    }

    impl FakeDocker {
        fn new(body: &str) -> Self {
            let directory = tempfile::tempdir().unwrap();
            let program = directory.path().join("fake docker");
            let log = directory.path().join("args");
            let cwd = directory.path().join("cwd");
            let script = format!(
                "#!/bin/sh\nprintf '%s\\000' \"$@\" > {}\npwd > {}\n{}\n",
                quote(&log),
                quote(&cwd),
                body,
            );
            fs::write(&program, script).unwrap();
            fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
            Self {
                adapter: DockerCli::with_program(program.into_os_string(), false),
                directory,
            }
        }

        fn args(&self) -> Vec<OsString> {
            fs::read(self.directory.path().join("args"))
                .unwrap()
                .split(|byte| *byte == 0)
                .filter(|value| !value.is_empty())
                .map(|value| OsString::from_vec(value.to_vec()))
                .collect()
        }

        fn request(&self) -> RunRequest {
            RunRequest {
                name: "zenv-project".into(),
                image: "example:latest".into(),
                command: vec!["sleep".into(), "365d".into()],
                options: BTreeMap::from([("detach".into(), OptionValue::Flag(true))]),
                directory: self.directory.path().to_path_buf(),
            }
        }
    }

    fn quote(path: &std::path::Path) -> String {
        format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"))
    }

    fn strings(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn inspect_matches_exact_name_and_parses_machine_state() {
        let fake = FakeDocker::new(
            r#"printf '%s\n' '{"ID":"first","Names":"zenv-project-other","State":"running"}' '{"ID":"second","Names":"zenv-project","State":"exited"}'"#,
        );
        assert_eq!(
            fake.adapter.inspect("zenv-project").unwrap(),
            Some(Container {
                id: "second".into(),
                name: "zenv-project".into(),
                state: ContainerState::Exited,
            })
        );
        assert_eq!(
            fake.args(),
            strings(&[
                "container",
                "ls",
                "--no-trunc",
                "--format",
                "{{json .}}",
                "--all"
            ])
        );
        assert!(fake.adapter.inspect("project").unwrap().is_none());
    }

    #[test]
    fn list_running_does_not_request_stopped_containers() {
        let fake = FakeDocker::new("exit 0");
        assert!(fake.adapter.list_running().unwrap().is_empty());
        assert!(!fake.args().contains(&OsString::from("--all")));
    }

    #[test]
    fn daemon_failure_is_not_a_missing_container() {
        let fake = FakeDocker::new("echo 'permission denied' >&2; exit 1");
        let error = fake
            .adapter
            .inspect("zenv-project")
            .unwrap_err()
            .to_string();
        assert!(error.contains("permission denied"));
        assert!(error.contains("exit code 1"));
    }

    #[test]
    fn malformed_responses_are_errors() {
        for body in [
            "echo not-json",
            "echo '{}'",
            "printf '%s' '{\"ID\":\"\",\"Names\":\"n\",\"State\":\"running\"}'",
        ] {
            let fake = FakeDocker::new(body);
            assert!(fake.adapter.inspect("n").is_err());
        }
    }

    #[test]
    fn exec_preserves_arguments_environment_and_option_types() {
        let fake = FakeDocker::new("exit 17");
        let command = vec![
            OsString::from("printf"),
            OsString::from("hello; $(touch x)"),
            OsString::from_vec(vec![0xff]),
        ];
        let request = ExecRequest {
            command: command.clone(),
            env: vec!["A=hello\nworld".into(), "B=x=y".into()],
            options: BTreeMap::from([
                ("interactive".into(), OptionValue::Flag(true)),
                ("tty".into(), OptionValue::Flag(false)),
                ("privileged".into(), OptionValue::Flag(true)),
                ("workdir".into(), OptionValue::Text("/a b".into())),
                ("user".into(), OptionValue::Integer(42)),
                (
                    "env-file".into(),
                    OptionValue::List(vec!["first".into(), "second".into()]),
                ),
            ]),
        };
        assert_eq!(fake.adapter.exec("container-id", &request).unwrap(), 17);
        let mut expected = strings(&[
            "exec",
            "--env-file",
            "first",
            "--env-file",
            "second",
            "--interactive",
            "--privileged",
            "--user",
            "42",
            "--workdir",
            "/a b",
            "--env",
            "A=hello\nworld",
            "--env",
            "B=x=y",
            "--",
            "container-id",
        ]);
        expected.extend(command);
        assert_eq!(fake.args(), expected);
    }

    #[test]
    fn exec_does_not_inherit_host_values_for_unset_entries() {
        let fake = FakeDocker::new("test -z \"${HOME+x}\"");
        let request = ExecRequest {
            command: strings(&["env"]),
            options: Options::new(),
            env: strings(&["HOME"]),
        };
        assert_eq!(fake.adapter.exec("container-id", &request).unwrap(), 0);
        assert!(
            fake.args()
                .windows(2)
                .any(|args| args == strings(&["--env", "HOME"]))
        );
    }

    #[test]
    fn strings_that_look_like_booleans_are_literal_values() {
        let fake = FakeDocker::new("exit 0");
        let request = ExecRequest {
            command: vec!["true".into()],
            env: vec![],
            options: BTreeMap::from([
                ("workdir".into(), OptionValue::Text("true".into())),
                ("user".into(), OptionValue::Text("false".into())),
            ]),
        };
        assert_eq!(fake.adapter.exec("id", &request).unwrap(), 0);
        assert_eq!(
            fake.args(),
            strings(&[
                "exec",
                "--user",
                "false",
                "--workdir",
                "true",
                "--",
                "id",
                "true"
            ])
        );
    }

    #[test]
    fn exec_reports_signal_exit_status() {
        let fake = FakeDocker::new("kill -TERM $$");
        assert_eq!(
            fake.adapter
                .exec(
                    "id",
                    &ExecRequest {
                        command: vec!["x".into()],
                        options: Options::new(),
                        env: vec![]
                    }
                )
                .unwrap(),
            143
        );
    }

    #[test]
    fn lifecycle_failures_are_not_ignored() {
        let fake = FakeDocker::new("echo broken >&2; exit 9");
        assert!(
            fake.adapter
                .start("id")
                .unwrap_err()
                .to_string()
                .contains("broken")
        );
        assert_eq!(fake.args(), strings(&["start", "--", "id"]));
        assert!(fake.adapter.stop("id").is_err());
        assert!(fake.adapter.remove("id", true).is_err());
        assert_eq!(fake.args(), strings(&["rm", "--force", "--", "id"]));
    }

    #[test]
    fn run_uses_private_cidfile_and_request_directory() {
        let fake = FakeDocker::new(&format!(
            "while [ $# -gt 0 ]; do if [ \"$1\" = --cidfile ]; then printf '%s' '{ID}' > \"$2\"; break; fi; shift; done"
        ));
        let request = fake.request();
        let container = fake.adapter.run(&request).unwrap();
        assert_eq!(container.id, ID);
        assert_eq!(container.name, request.name);
        let args = fake.args();
        assert_eq!(
            &args[..4],
            strings(&["run", "--name", "zenv-project", "--cidfile"])
        );
        assert!(!std::path::Path::new(&args[4]).exists());
        assert_eq!(
            &args[5..],
            strings(&["--detach", "--", "example:latest", "sleep", "365d"])
        );
        let cwd = fs::read_to_string(fake.directory.path().join("cwd")).unwrap();
        assert_eq!(
            std::path::Path::new(cwd.trim()),
            request.directory.canonicalize().unwrap()
        );
    }

    #[test]
    fn run_failure_cleans_only_the_id_created_by_that_invocation() {
        let fake = FakeDocker::new(&format!(
            "if [ \"$1\" = run ]; then\nwhile [ $# -gt 0 ]; do if [ \"$1\" = --cidfile ]; then printf '%s' '{ID}' > \"$2\"; break; fi; shift; done\nexit 7\nfi\nexit 0"
        ));
        let error = fake.adapter.run(&fake.request()).unwrap_err();
        assert!(error.to_string().contains("exit code 7"));
        assert_eq!(fake.args(), strings(&["rm", "--force", "--", ID]));
    }

    #[test]
    fn failed_run_without_cidfile_never_removes_an_existing_container() {
        let fake = FakeDocker::new("exit 4");
        assert!(fake.adapter.run(&fake.request()).is_err());
        assert_eq!(fake.args()[0], "run");
    }

    #[test]
    fn run_reports_both_creation_and_cleanup_failures() {
        let fake = FakeDocker::new(&format!(
            "if [ \"$1\" = run ]; then\nwhile [ $# -gt 0 ]; do if [ \"$1\" = --cidfile ]; then printf '%s' '{ID}' > \"$2\"; break; fi; shift; done\nexit 7\nfi\necho cleanup-broken >&2\nexit 8"
        ));
        let error = fake.adapter.run(&fake.request()).unwrap_err().to_string();
        assert!(error.contains("exit code 7"));
        assert!(error.contains("cleanup-broken"));
    }

    #[test]
    fn managed_and_invalid_options_are_rejected_before_launch() {
        let fake = FakeDocker::new("exit 0");
        for (name, value) in [
            ("cidfile", "some-file"),
            ("name", "other"),
            ("bad flag", "value"),
        ] {
            let mut request = fake.request();
            request
                .options
                .insert(name.into(), OptionValue::Text(value.into()));
            assert!(fake.adapter.run(&request).is_err());
        }
        assert!(!fake.directory.path().join("args").exists());
    }

    #[test]
    fn successful_run_requires_a_valid_container_id() {
        let fake = FakeDocker::new("exit 0");
        assert!(
            fake.adapter
                .run(&fake.request())
                .unwrap_err()
                .to_string()
                .contains("valid container ID")
        );
    }
}
