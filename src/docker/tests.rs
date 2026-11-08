use super::*;
use crate::runtime::{
    CONFIG_HASH_LABEL, Container, ContainerRuntime, ContainerState, ExecRequest, OptionValue,
    Options, RunRequest,
};
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
            fingerprint: None,
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
        r#"if [ "$2" = inspect ]; then printf '%s\n' '{"zenv.config-hash":"saved-hash"}'; else printf '%s\n' '{"ID":"first","Names":"zenv-project-other","State":"running"}' '{"ID":"second","Names":"zenv-project","State":"exited"}'; fi"#,
    );
    assert_eq!(
        fake.adapter.inspect("zenv-project").unwrap(),
        Some(Container {
            id: "second".into(),
            name: "zenv-project".into(),
            state: ContainerState::Exited,
            fingerprint: Some("saved-hash".into()),
        })
    );
    assert_eq!(
        fake.args(),
        strings(&[
            "container",
            "inspect",
            "--format",
            "{{json .Config.Labels}}",
            "--",
            "second",
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
        assert!(fake.adapter.validate_run(&request).is_err());
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

#[test]
fn invalid_exec_options_are_rejected_without_launching_docker() {
    let fake = FakeDocker::new("exit 0");
    let request = ExecRequest {
        command: strings(&["work"]),
        options: BTreeMap::from([("bad flag".into(), OptionValue::Flag(true))]),
        env: vec![],
    };
    assert!(fake.adapter.validate_exec(&request).is_err());
    assert!(!fake.directory.path().join("args").exists());
}

#[test]
fn run_preserves_user_labels_and_stores_the_fingerprint_last() {
    for labels in [
        OptionValue::Text("team=tools".into()),
        OptionValue::List(vec!["team=tools".into(), "zenv.config-hash=old".into()]),
    ] {
        let fake = FakeDocker::new(&format!(
            "while [ $# -gt 0 ]; do if [ \"$1\" = --cidfile ]; then printf '%s' '{ID}' > \"$2\"; break; fi; shift; done"
        ));
        let mut request = fake.request();
        request.fingerprint = Some("declared-hash".into());
        request.options.insert("label".into(), labels.clone());
        let container = fake.adapter.run(&request).unwrap();
        assert_eq!(container.fingerprint, request.fingerprint);
        assert_eq!(request.options["label"], labels);
        let args = fake.args();
        let saved: Vec<_> = args
            .windows(2)
            .filter(|pair| pair[0] == "--label")
            .map(|pair| pair[1].clone())
            .collect();
        let mut expected: Vec<OsString> = match labels {
            OptionValue::Text(label) => vec![label.into()],
            OptionValue::List(labels) => labels.into_iter().map(OsString::from).collect(),
            _ => unreachable!(),
        };
        expected.push(format!("{CONFIG_HASH_LABEL}=declared-hash").into());
        assert_eq!(saved, expected);
    }
}

#[test]
fn inspect_handles_missing_null_and_stored_fingerprints() {
    for (labels, fingerprint) in [
        ("null", None),
        ("{}", None),
        ("{\"other\":\"label\"}", None),
        ("{\"zenv.config-hash\":\"old\"}", Some("old".to_owned())),
    ] {
        let fake = FakeDocker::new(&format!(
            "if [ \"$2\" = inspect ]; then printf '%s\\n' '{labels}'; else printf '%s\\n' '{{\"ID\":\"id\",\"Names\":\"project\",\"State\":\"running\"}}'; fi"
        ));
        assert_eq!(
            fake.adapter
                .inspect("project")
                .unwrap()
                .unwrap()
                .fingerprint,
            fingerprint
        );
    }
}

#[test]
fn label_inspection_errors_are_not_treated_as_missing_containers() {
    for response in [
        "echo denied >&2; exit 9",
        "echo not-json",
        "echo '[]'",
        "echo '{\"zenv.config-hash\":false}'",
    ] {
        let fake = FakeDocker::new(&format!(
            "if [ \"$2\" = inspect ]; then {response}; else printf '%s\\n' '{{\"ID\":\"id\",\"Names\":\"project\",\"State\":\"running\"}}'; fi"
        ));
        assert!(fake.adapter.inspect("project").is_err());
        assert_eq!(
            fake.args(),
            strings(&[
                "container",
                "inspect",
                "--format",
                "{{json .Config.Labels}}",
                "--",
                "id",
            ])
        );
    }
}

#[test]
fn networks_attempt_every_name_without_checks_and_ignore_errors() {
    let directory = tempfile::tempdir().unwrap();
    let calls = directory.path().join("network-calls");
    let fake = FakeDocker::new(&format!(
        "printf '%s\\n' \"$*\" >> {}\necho ignored\necho ignored >&2\nexit 9",
        quote(&calls),
    ));
    let names: Vec<String> = [
        "project",
        "project",
        "bridge",
        "host",
        "none",
        "default",
        "container:other",
        "name=project,alias=app",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    fake.adapter.create_networks(&names);
    assert_eq!(
        fake.args(),
        strings(&[
            "network",
            "create",
            "--label",
            "zenv.managed-network=true",
            "--",
            "name=project,alias=app",
        ])
    );
    fake.adapter.remove_networks(&names);
    assert_eq!(
        fake.args(),
        strings(&["network", "rm", "--", "name=project,alias=app"])
    );
    assert_eq!(
        fs::read_to_string(calls).unwrap().lines().count(),
        names.len() * 2
    );
}

#[test]
fn no_configured_networks_do_not_launch_docker() {
    let fake = FakeDocker::new("exit 99");
    fake.adapter.create_networks(&[]);
    fake.adapter.remove_networks(&[]);
    assert!(!fake.directory.path().join("args").exists());
}
