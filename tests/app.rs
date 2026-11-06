#![forbid(unsafe_code)]

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, VecDeque},
    ffi::OsString,
};

use tempfile::TempDir;
use zenv::{
    Error, Result, app,
    config::{Config, HostContext},
    runtime::{Container, ContainerRuntime, ContainerState, ExecRequest, RunRequest},
};

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Inspect(String),
    Run(String),
    Start(String),
    Exec(String, Vec<OsString>),
    Stop(String),
    Remove(String, bool),
    List,
}

struct FakeRuntime {
    events: RefCell<Vec<Event>>,
    inspected: RefCell<Option<Result<Option<Container>>>>,
    exec_results: RefCell<VecDeque<Result<i32>>>,
    running: Vec<Container>,
    fail_run: bool,
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
            Ok(container(
                "owned-id",
                &request.name,
                ContainerState::Running,
            ))
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
}

fn container(id: &str, name: &str, state: ContainerState) -> Container {
    Container {
        id: id.into(),
        name: name.into(),
        state,
    }
}

fn fixture() -> (TempDir, Config, HostContext) {
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
    let config = Config::load(None, &host).unwrap();
    (directory, config, host)
}

fn exec(id: &str, command: &[&str]) -> Event {
    Event::Exec(id.into(), command.iter().map(OsString::from).collect())
}

#[test]
fn creates_initializes_and_executes_in_order() {
    let (_directory, config, host) = fixture();
    let runtime = FakeRuntime::new(None);
    runtime
        .exec_results
        .borrow_mut()
        .extend([Ok(0), Ok(0), Ok(37)]);
    assert_eq!(
        app::execute(&runtime, &config, &["work".into(), "a b".into()], &host).unwrap(),
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
    let (_directory, config, host) = fixture();
    let runtime = FakeRuntime::new(Some(container(
        "existing",
        "zenv-test",
        ContainerState::Running,
    )));
    app::execute(&runtime, &config, &["work".into()], &host).unwrap();
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
    let (_directory, config, host) = fixture();
    for state in [ContainerState::Exited, ContainerState::Created] {
        let runtime = FakeRuntime::new(Some(container("existing", "zenv-test", state)));
        app::execute(&runtime, &config, &["work".into()], &host).unwrap();
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
    let (_directory, config, host) = fixture();
    let mut runtime = FakeRuntime::new(Some(container(
        "existing",
        "zenv-test",
        ContainerState::Exited,
    )));
    runtime.fail_start = true;
    assert!(
        app::execute(&runtime, &config, &["work".into()], &host)
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
    let (_directory, config, host) = fixture();
    for state in [
        ContainerState::Paused,
        ContainerState::Restarting,
        ContainerState::Dead,
        ContainerState::Unknown("future".into()),
    ] {
        let runtime = FakeRuntime::new(Some(container("existing", "zenv-test", state)));
        assert!(app::execute(&runtime, &config, &["work".into()], &host).is_err());
        assert_eq!(
            *runtime.events.borrow(),
            [Event::Inspect("zenv-test".into())]
        );
    }
}

#[test]
fn failed_initialization_removes_only_the_new_container() {
    let (_directory, config, host) = fixture();
    for failure in [Ok(12), Err(Error::runtime("exec", "cannot execute init"))] {
        let runtime = FakeRuntime::new(None);
        runtime.exec_results.borrow_mut().push_back(failure);
        assert!(app::execute(&runtime, &config, &["work".into()], &host).is_err());
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
    let (_directory, config, host) = fixture();
    let mut runtime = FakeRuntime::new(None);
    runtime.fail_remove = true;
    runtime.exec_results.borrow_mut().push_back(Ok(12));
    let error = app::execute(&runtime, &config, &["work".into()], &host)
        .unwrap_err()
        .to_string();
    assert!(error.contains("12"), "{error}");
    assert!(error.contains("cleanup failed"), "{error}");
}

#[test]
fn inspect_failure_does_not_create_a_container() {
    let (_directory, config, host) = fixture();
    let runtime = FakeRuntime::new(None);
    *runtime.inspected.borrow_mut() = Some(Err(Error::runtime("inspect", "daemon unavailable")));
    assert!(app::execute(&runtime, &config, &["work".into()], &host).is_err());
    assert_eq!(
        *runtime.events.borrow(),
        [Event::Inspect("zenv-test".into())]
    );
}

#[test]
fn creation_failure_prevents_init_and_user_command() {
    let (_directory, config, host) = fixture();
    let mut runtime = FakeRuntime::new(None);
    runtime.fail_run = true;
    assert!(app::execute(&runtime, &config, &["work".into()], &host).is_err());
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
    let (_directory, config, host) = fixture();
    let runtime = FakeRuntime::new(None);
    runtime
        .exec_results
        .borrow_mut()
        .extend([Ok(0), Ok(0), Ok(9)]);
    assert_eq!(
        app::execute(&runtime, &config, &["work".into()], &host).unwrap(),
        9
    );
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
    let runtime = FakeRuntime::new(Some(container(
        "existing",
        "zenv-test",
        ContainerState::Running,
    )));
    app::remove(&runtime, "zenv-test").unwrap();
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
    let mut runtime = FakeRuntime::new(Some(container(
        "existing",
        "zenv-test",
        ContainerState::Running,
    )));
    runtime.fail_stop.insert("existing".into());
    assert!(app::remove(&runtime, "zenv-test").is_err());
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
    let runtime = FakeRuntime::new(None);
    app::remove(&runtime, "missing").unwrap();
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
