#![forbid(unsafe_code)]

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, VecDeque},
    ffi::OsString,
};

use tempfile::TempDir;
use zenv::{
    Error, Result, app,
    config::{Alias, Config, ConfigLoader, FileConfigLoader, HostContext},
    runtime::{Container, ContainerRuntime, ContainerState, ExecRequest, OptionValue, RunRequest},
};

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Inspect(String),
    Run(String),
    Start(String),
    Exec(String, Vec<OsString>),
    Stop(String),
    Remove(String, bool),
    CreateNetworks(Vec<String>),
    RemoveNetworks(Vec<String>),
    Confirm,
    List,
}

struct FakeRuntime {
    events: RefCell<Vec<Event>>,
    inspected: RefCell<Option<Result<Option<Container>>>>,
    exec_results: RefCell<VecDeque<Result<i32>>>,
    running: Vec<Container>,
    fail_run: bool,
    fail_run_validation: bool,
    invalid_exec_command: Option<String>,
    fail_start: bool,
    fail_remove: bool,
    fail_stop: BTreeSet<String>,
}

impl FakeRuntime {
    fn new(container: Option<Container>) -> Self {
        Self {
            events: RefCell::new(Vec::new()),
            inspected: RefCell::new(Some(Ok(container))),
            exec_results: RefCell::new(VecDeque::new()),
            running: Vec::new(),
            fail_run: false,
            fail_run_validation: false,
            invalid_exec_command: None,
            fail_start: false,
            fail_remove: false,
            fail_stop: BTreeSet::new(),
        }
    }

    fn record(&self, event: Event) {
        self.events.borrow_mut().push(event);
    }
}

impl ContainerRuntime for FakeRuntime {
    fn validate_run(&self, _request: &RunRequest) -> Result<()> {
        if self.fail_run_validation {
            Err(Error::Config("invalid creation options".into()))
        } else {
            Ok(())
        }
    }

    fn validate_exec(&self, request: &ExecRequest) -> Result<()> {
        if self
            .invalid_exec_command
            .as_ref()
            .is_some_and(|name| request.command.first() == Some(&OsString::from(name)))
        {
            Err(Error::Config("invalid execution options".into()))
        } else {
            Ok(())
        }
    }

    fn inspect(&self, name: &str) -> Result<Option<Container>> {
        self.record(Event::Inspect(name.into()));
        self.inspected
            .borrow_mut()
            .take()
            .expect("one inspect expected")
    }

    fn run(&self, request: &RunRequest) -> Result<Container> {
        self.record(Event::Run(request.name.clone()));
        if self.fail_run {
            Err(Error::runtime("run", "creation failed"))
        } else {
            let mut created = container("owned-id", &request.name, ContainerState::Running);
            created.fingerprint = request.fingerprint.clone();
            Ok(created)
        }
    }

    fn start(&self, id: &str) -> Result<()> {
        self.record(Event::Start(id.into()));
        if self.fail_start {
            Err(Error::runtime("start", "start failed"))
        } else {
            Ok(())
        }
    }

    fn exec(&self, id: &str, request: &ExecRequest) -> Result<i32> {
        self.record(Event::Exec(id.into(), request.command.clone()));
        self.exec_results.borrow_mut().pop_front().unwrap_or(Ok(0))
    }

    fn stop(&self, id: &str) -> Result<()> {
        self.record(Event::Stop(id.into()));
        if self.fail_stop.contains(id) {
            Err(Error::runtime("stop", "stop failed"))
        } else {
            Ok(())
        }
    }

    fn remove(&self, id: &str, force: bool) -> Result<()> {
        self.record(Event::Remove(id.into(), force));
        if self.fail_remove {
            Err(Error::runtime("remove", "cleanup failed"))
        } else {
            Ok(())
        }
    }

    fn list_running(&self) -> Result<Vec<Container>> {
        self.record(Event::List);
        Ok(self.running.clone())
    }

    fn create_networks(&self, names: &[String]) {
        if !names.is_empty() {
            self.record(Event::CreateNetworks(names.to_vec()));
        }
    }

    fn remove_networks(&self, names: &[String]) {
        if !names.is_empty() {
            self.record(Event::RemoveNetworks(names.to_vec()));
        }
    }
}

fn container(id: &str, name: &str, state: ContainerState) -> Container {
    Container {
        id: id.into(),
        name: name.into(),
        state,
        fingerprint: None,
    }
}

fn fixture() -> (TempDir, Config) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("Zenvfile"),
        r#"
[main]
image = "ubuntu:latest"
name = "zenv-test"
[run]
command = ["sleep", "365d"]
init_commands = [["init-one"], ["init-two"]]
"#,
    )
    .unwrap();
    let host = HostContext {
        cwd: directory.path().to_owned(),
        uid: 1000,
        gid: 1000,
        tty: false,
        environment: BTreeMap::new(),
    };
    let mut config = FileConfigLoader::new(&host).load(None).unwrap();
    config.run.check_zenvfile_changes = false;
    (directory, config)
}

fn exec(id: &str, command: &[&str]) -> Event {
    Event::Exec(id.into(), command.iter().map(OsString::from).collect())
}

fn execute(runtime: &FakeRuntime, config: &Config, command: &[OsString]) -> Result<i32> {
    app::execute(runtime, config, command, &mut || {
        panic!("confirmation is unexpected in this lifecycle test")
    })
}

#[test]
fn creates_initializes_and_executes_in_order() {
    let (_directory, config) = fixture();
    let runtime = FakeRuntime::new(None);
    runtime
        .exec_results
        .borrow_mut()
        .extend([Ok(0), Ok(0), Ok(37)]);
    assert_eq!(
        execute(&runtime, &config, &["work".into(), "a b".into()]).unwrap(),
        37
    );
    assert_eq!(
        *runtime.events.borrow(),
        [
            Event::Inspect("zenv-test".into()),
            Event::Run("zenv-test".into()),
            exec("owned-id", &["init-one"]),
            exec("owned-id", &["init-two"]),
            exec("owned-id", &["work", "a b"]),
        ]
    );
}

#[test]
fn running_container_does_not_repeat_initialization() {
    let (_directory, config) = fixture();
    let runtime = FakeRuntime::new(Some(container(
        "existing",
        "zenv-test",
        ContainerState::Running,
    )));
    execute(&runtime, &config, &["work".into()]).unwrap();
    assert_eq!(
        *runtime.events.borrow(),
        [
            Event::Inspect("zenv-test".into()),
            exec("existing", &["work"])
        ]
    );
}

#[test]
fn starts_exited_and_created_containers_without_initializing_again() {
    let (_directory, config) = fixture();
    for state in [ContainerState::Exited, ContainerState::Created] {
        let runtime = FakeRuntime::new(Some(container("existing", "zenv-test", state)));
        execute(&runtime, &config, &["work".into()]).unwrap();
        assert_eq!(
            *runtime.events.borrow(),
            [
                Event::Inspect("zenv-test".into()),
                Event::Start("existing".into()),
                exec("existing", &["work"])
            ]
        );
    }
}

#[test]
fn start_failure_prevents_execution() {
    let (_directory, config) = fixture();
    let mut runtime = FakeRuntime::new(Some(container(
        "existing",
        "zenv-test",
        ContainerState::Exited,
    )));
    runtime.fail_start = true;
    assert!(
        execute(&runtime, &config, &["work".into()])
            .unwrap_err()
            .to_string()
            .contains("start failed")
    );
    assert_eq!(
        *runtime.events.borrow(),
        [
            Event::Inspect("zenv-test".into()),
            Event::Start("existing".into())
        ]
    );
}

#[test]
fn unavailable_states_prevent_execution() {
    let (_directory, config) = fixture();
    for state in [
        ContainerState::Paused,
        ContainerState::Restarting,
        ContainerState::Dead,
        ContainerState::Unknown("future".into()),
    ] {
        let runtime = FakeRuntime::new(Some(container("existing", "zenv-test", state)));
        assert!(execute(&runtime, &config, &["work".into()]).is_err());
        assert_eq!(
            *runtime.events.borrow(),
            [Event::Inspect("zenv-test".into())]
        );
    }
}

#[test]
fn failed_initialization_removes_only_the_new_container() {
    let (_directory, config) = fixture();
    for failure in [Ok(12), Err(Error::runtime("exec", "cannot execute init"))] {
        let runtime = FakeRuntime::new(None);
        runtime.exec_results.borrow_mut().push_back(failure);
        assert!(execute(&runtime, &config, &["work".into()]).is_err());
        assert_eq!(
            *runtime.events.borrow(),
            [
                Event::Inspect("zenv-test".into()),
                Event::Run("zenv-test".into()),
                exec("owned-id", &["init-one"]),
                Event::Remove("owned-id".into(), true),
            ]
        );
    }
}

#[test]
fn reports_initialization_and_cleanup_failures_together() {
    let (_directory, config) = fixture();
    let mut runtime = FakeRuntime::new(None);
    runtime.fail_remove = true;
    runtime.exec_results.borrow_mut().push_back(Ok(12));
    let error = execute(&runtime, &config, &["work".into()])
        .unwrap_err()
        .to_string();
    assert!(error.contains("12"), "{error}");
    assert!(error.contains("cleanup failed"), "{error}");
}

#[test]
fn inspect_failure_does_not_create_a_container() {
    let (_directory, config) = fixture();
    let runtime = FakeRuntime::new(None);
    *runtime.inspected.borrow_mut() = Some(Err(Error::runtime("inspect", "daemon unavailable")));
    assert!(execute(&runtime, &config, &["work".into()]).is_err());
    assert_eq!(
        *runtime.events.borrow(),
        [Event::Inspect("zenv-test".into())]
    );
}

#[test]
fn creation_failure_prevents_init_and_user_command() {
    let (_directory, config) = fixture();
    let mut runtime = FakeRuntime::new(None);
    runtime.fail_run = true;
    assert!(execute(&runtime, &config, &["work".into()]).is_err());
    assert_eq!(
        *runtime.events.borrow(),
        [
            Event::Inspect("zenv-test".into()),
            Event::Run("zenv-test".into())
        ]
    );
}

#[test]
fn user_command_failure_does_not_remove_the_container() {
    let (_directory, config) = fixture();
    let runtime = FakeRuntime::new(None);
    runtime
        .exec_results
        .borrow_mut()
        .extend([Ok(0), Ok(0), Ok(9)]);
    assert_eq!(execute(&runtime, &config, &["work".into()]).unwrap(), 9);
    assert!(
        !runtime
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, Event::Remove(..)))
    );
}

#[test]
fn removes_running_container_after_stopping_it() {
    let (_directory, config) = fixture();
    let runtime = FakeRuntime::new(Some(container(
        "existing",
        "zenv-test",
        ContainerState::Running,
    )));
    app::remove(&runtime, &config).unwrap();
    assert_eq!(
        *runtime.events.borrow(),
        [
            Event::Inspect("zenv-test".into()),
            Event::Stop("existing".into()),
            Event::Remove("existing".into(), false)
        ]
    );
}

#[test]
fn removal_does_not_continue_after_stop_failure() {
    let (_directory, config) = fixture();
    let mut runtime = FakeRuntime::new(Some(container(
        "existing",
        "zenv-test",
        ContainerState::Running,
    )));
    runtime.fail_stop.insert("existing".into());
    assert!(app::remove(&runtime, &config).is_err());
    assert_eq!(
        *runtime.events.borrow(),
        [
            Event::Inspect("zenv-test".into()),
            Event::Stop("existing".into())
        ]
    );
}

#[test]
fn removing_or_stopping_an_absent_container_is_idempotent() {
    let (_directory, mut config) = fixture();
    config.main.name = "missing".into();
    let runtime = FakeRuntime::new(None);
    app::remove(&runtime, &config).unwrap();
    assert_eq!(*runtime.events.borrow(), [Event::Inspect("missing".into())]);
    let runtime = FakeRuntime::new(None);
    app::stop(&runtime, "missing").unwrap();
    assert_eq!(*runtime.events.borrow(), [Event::Inspect("missing".into())]);
}

#[test]
fn stop_all_filters_names_excludes_current_and_collects_failures() {
    let mut runtime = FakeRuntime::new(None);
    runtime.running = vec![
        container("current", "zenv-current", ContainerState::Running),
        container("other", "other-zenv-project", ContainerState::Running),
        container("first", "zenv-first", ContainerState::Running),
        container("second", "zenv-second", ContainerState::Running),
        container("third", "zenv-third", ContainerState::Running),
    ];
    runtime.fail_stop.extend(["first".into(), "third".into()]);
    let error = app::stop_all(&runtime, Some("zenv-current"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("zenv-first"), "{error}");
    assert!(error.contains("zenv-third"), "{error}");
    assert_eq!(
        *runtime.events.borrow(),
        [
            Event::List,
            Event::Stop("first".into()),
            Event::Stop("second".into()),
            Event::Stop("third".into())
        ]
    );
}

#[test]
fn matching_fingerprint_never_requests_confirmation() {
    let (_directory, mut config) = fixture();
    config.run.check_zenvfile_changes = true;
    for state in [
        ContainerState::Running,
        ContainerState::Exited,
        ContainerState::Created,
    ] {
        let mut existing = container("existing", "zenv-test", state.clone());
        existing.fingerprint = Some(config.fingerprint.clone());
        let runtime = FakeRuntime::new(Some(existing));
        assert_eq!(execute(&runtime, &config, &["work".into()]).unwrap(), 0);
        let mut expected = vec![Event::Inspect("zenv-test".into())];
        if state != ContainerState::Running {
            expected.push(Event::Start("existing".into()));
        }
        expected.push(exec("existing", &["work"]));
        assert_eq!(*runtime.events.borrow(), expected);
    }
}

#[test]
fn disabled_check_ignores_missing_and_outdated_fingerprints() {
    let (_directory, config) = fixture();
    for fingerprint in [None, Some("outdated".into())] {
        let mut existing = container("existing", "zenv-test", ContainerState::Running);
        existing.fingerprint = fingerprint;
        let runtime = FakeRuntime::new(Some(existing));
        assert_eq!(execute(&runtime, &config, &["work".into()]).unwrap(), 0);
        assert_eq!(
            *runtime.events.borrow(),
            [
                Event::Inspect("zenv-test".into()),
                exec("existing", &["work"]),
            ]
        );
    }
}

#[test]
fn declining_recreation_keeps_existing_container_and_starts_it_if_needed() {
    let (_directory, mut config) = fixture();
    config.run.check_zenvfile_changes = true;
    for state in [
        ContainerState::Running,
        ContainerState::Exited,
        ContainerState::Created,
    ] {
        for fingerprint in [None, Some("outdated".into())] {
            let mut existing = container("existing", "zenv-test", state.clone());
            existing.fingerprint = fingerprint;
            let runtime = FakeRuntime::new(Some(existing));
            let mut confirm = || {
                runtime.record(Event::Confirm);
                Ok(false)
            };
            assert_eq!(
                app::execute(&runtime, &config, &["work".into()], &mut confirm).unwrap(),
                0
            );
            let mut expected = vec![Event::Inspect("zenv-test".into()), Event::Confirm];
            if state != ContainerState::Running {
                expected.push(Event::Start("existing".into()));
            }
            expected.push(exec("existing", &["work"]));
            assert_eq!(*runtime.events.borrow(), expected);
        }
    }
}

#[test]
fn approved_recreation_preserves_alias_arguments_exit_code_and_network_order() {
    let (_directory, mut config) = fixture();
    config.run.check_zenvfile_changes = true;
    config
        .run
        .options
        .insert("network".into(), OptionValue::Text("project-dev".into()));
    config.aliases.insert(
        "work".into(),
        Alias {
            command: vec!["tool".into(), "alias arg".into()],
            description: None,
            exec: config.exec.clone(),
        },
    );
    for exit_code in [0, 27] {
        let runtime = FakeRuntime::new(Some(container(
            "existing",
            "zenv-test",
            ContainerState::Running,
        )));
        runtime
            .exec_results
            .borrow_mut()
            .extend([Ok(0), Ok(0), Ok(exit_code)]);
        let mut confirm = || {
            runtime.record(Event::Confirm);
            Ok(true)
        };
        assert_eq!(
            app::execute(
                &runtime,
                &config,
                &["work".into(), "a b".into(), "--flag".into()],
                &mut confirm
            )
            .unwrap(),
            exit_code
        );
        assert_eq!(
            *runtime.events.borrow(),
            [
                Event::Inspect("zenv-test".into()),
                Event::Confirm,
                Event::Stop("existing".into()),
                Event::Remove("existing".into(), false),
                Event::RemoveNetworks(vec!["project-dev".into()]),
                Event::CreateNetworks(vec!["project-dev".into()]),
                Event::Run("zenv-test".into()),
                exec("owned-id", &["init-one"]),
                exec("owned-id", &["init-two"]),
                exec("owned-id", &["tool", "alias arg", "a b", "--flag"]),
            ]
        );
    }
}

#[test]
fn confirmation_error_prevents_all_mutations() {
    let (_directory, mut config) = fixture();
    config.run.check_zenvfile_changes = true;
    let runtime = FakeRuntime::new(Some(container(
        "existing",
        "zenv-test",
        ContainerState::Exited,
    )));
    let error = app::execute(&runtime, &config, &["work".into()], &mut || {
        Err(Error::Config("confirmation aborted".into()))
    })
    .unwrap_err();
    assert!(error.to_string().contains("confirmation aborted"));
    assert_eq!(
        *runtime.events.borrow(),
        [Event::Inspect("zenv-test".into())]
    );
}

#[test]
fn invalid_command_cannot_remove_a_container_during_recreation() {
    let (_directory, mut config) = fixture();
    config.run.check_zenvfile_changes = true;
    let runtime = FakeRuntime::new(Some(container(
        "existing",
        "zenv-test",
        ContainerState::Running,
    )));
    assert!(app::execute(&runtime, &config, &[], &mut || Ok(true)).is_err());
    assert!(runtime.events.borrow().is_empty());
}

#[test]
fn invalid_replacement_requests_never_remove_existing_container_or_networks() {
    let (_directory, mut config) = fixture();
    config.run.check_zenvfile_changes = true;
    config
        .run
        .options
        .insert("network".into(), OptionValue::Text("project-dev".into()));
    for invalid_init in [false, true] {
        let mut runtime = FakeRuntime::new(Some(container(
            "existing",
            "zenv-test",
            ContainerState::Running,
        )));
        runtime.fail_run_validation = !invalid_init;
        runtime.invalid_exec_command = invalid_init.then(|| "init-one".into());
        assert!(app::execute(&runtime, &config, &["work".into()], &mut || Ok(true)).is_err());
        assert_eq!(
            *runtime.events.borrow(),
            [Event::Inspect("zenv-test".into())]
        );
    }
}

#[test]
fn failed_removal_during_recreation_prevents_network_cleanup_and_creation() {
    let (_directory, mut config) = fixture();
    config.run.check_zenvfile_changes = true;
    config
        .run
        .options
        .insert("network".into(), OptionValue::Text("project-dev".into()));
    let mut runtime = FakeRuntime::new(Some(container(
        "existing",
        "zenv-test",
        ContainerState::Exited,
    )));
    runtime.fail_remove = true;
    assert!(app::execute(&runtime, &config, &["work".into()], &mut || Ok(true)).is_err());
    assert_eq!(
        *runtime.events.borrow(),
        [
            Event::Inspect("zenv-test".into()),
            Event::Remove("existing".into(), false),
        ]
    );
}

#[test]
fn configured_networks_are_created_before_container_and_removed_after_it() {
    let (_directory, mut config) = fixture();
    config.run.options.insert(
        "network".into(),
        OptionValue::List(vec!["first".into(), "second".into()]),
    );
    let runtime = FakeRuntime::new(None);
    execute(&runtime, &config, &["work".into()]).unwrap();
    assert_eq!(
        &runtime.events.borrow()[..3],
        [
            Event::Inspect("zenv-test".into()),
            Event::CreateNetworks(vec!["first".into(), "second".into()]),
            Event::Run("zenv-test".into()),
        ]
    );
    let runtime = FakeRuntime::new(Some(container(
        "existing",
        "zenv-test",
        ContainerState::Running,
    )));
    app::remove(&runtime, &config).unwrap();
    assert_eq!(
        *runtime.events.borrow(),
        [
            Event::Inspect("zenv-test".into()),
            Event::Stop("existing".into()),
            Event::Remove("existing".into(), false),
            Event::RemoveNetworks(vec!["first".into(), "second".into()]),
        ]
    );
}

#[test]
fn networks_are_removed_even_when_container_is_absent() {
    let (_directory, mut config) = fixture();
    config
        .run
        .options
        .insert("network".into(), OptionValue::Text("project-dev".into()));
    let runtime = FakeRuntime::new(None);
    app::remove(&runtime, &config).unwrap();
    assert_eq!(
        *runtime.events.borrow(),
        [
            Event::Inspect("zenv-test".into()),
            Event::RemoveNetworks(vec!["project-dev".into()]),
        ]
    );
}

#[test]
fn disabled_network_management_has_no_network_operations() {
    let (_directory, mut config) = fixture();
    config
        .run
        .options
        .insert("network".into(), OptionValue::Text("project-dev".into()));
    config.run.manage_network = false;
    let runtime = FakeRuntime::new(None);
    execute(&runtime, &config, &["work".into()]).unwrap();
    assert!(
        !runtime
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, Event::CreateNetworks(_) | Event::RemoveNetworks(_)))
    );
    let runtime = FakeRuntime::new(None);
    app::remove(&runtime, &config).unwrap();
    assert_eq!(
        *runtime.events.borrow(),
        [Event::Inspect("zenv-test".into())]
    );
}

#[test]
fn creation_failure_does_not_roll_back_networks() {
    let (_directory, mut config) = fixture();
    config
        .run
        .options
        .insert("network".into(), OptionValue::Text("project-dev".into()));
    let mut runtime = FakeRuntime::new(None);
    runtime.fail_run = true;
    assert!(execute(&runtime, &config, &["work".into()]).is_err());
    assert_eq!(
        *runtime.events.borrow(),
        [
            Event::Inspect("zenv-test".into()),
            Event::CreateNetworks(vec!["project-dev".into()]),
            Event::Run("zenv-test".into()),
        ]
    );
}
