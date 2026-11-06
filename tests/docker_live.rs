#![forbid(unsafe_code)]

//! Opt-in checks against a real Docker daemon. Every container is tracked by ID.

use std::{
    cell::RefCell,
    ffi::OsString,
    fs,
    io::Write,
    os::unix::fs::MetadataExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use zenv::{
    Result, app,
    config::{Config, HostContext},
    docker::DockerCli,
    runtime::{Container, ContainerRuntime, ContainerState, ExecRequest, RunRequest},
};

struct Sandbox {
    directory: tempfile::TempDir,
    runtime: DockerCli,
    ids: RefCell<Vec<String>>,
    host: HostContext,
    config: Config,
}

impl Sandbox {
    fn new(failing_init: bool) -> Self {
        let root = std::env::var_os("ZENV_TEST_TMPDIR").map(PathBuf::from);
        if let Some(root) = &root {
            fs::create_dir_all(root).unwrap();
        }
        let directory = match root {
            Some(root) => tempfile::Builder::new()
                .prefix("zenv-live-")
                .tempdir_in(root),
            None => tempfile::Builder::new().prefix("zenv-live-").tempdir(),
        }
        .unwrap();
        let mut host = HostContext::capture().unwrap();
        host.cwd = directory.path().join("nested");
        fs::create_dir(&host.cwd).unwrap();
        host.tty = false;
        host.environment.clear();
        host.environment
            .insert("ZENV_LIVE_VALUE".into(), "one\ntwo".into());
        let name = directory.path().file_name().unwrap().to_str().unwrap();
        let image = std::env::var("ZENV_TEST_IMAGE").unwrap_or_else(|_| "ubuntu:24.04".into());
        let mut source = format!("[main]\nname = {name:?}\nimage = {image:?}\n");
        if failing_init {
            source.push_str("[run]\ninit_commands = [['sh', '-c', 'exit 17']]\n");
        }
        source.push_str("[exec]\nenv_excludes = ['PATH', 'HOME', 'USER', 'SHELL']\n");
        fs::write(directory.path().join("Zenvfile"), source).unwrap();
        let config = Config::load(None, &host).unwrap();
        Self {
            directory,
            runtime: DockerCli::new(false),
            ids: RefCell::new(vec![]),
            host,
            config,
        }
    }

    fn execute(&self, args: &[&str]) -> i32 {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        app::execute(self, &self.config, &args, &self.host).unwrap()
    }
}

impl ContainerRuntime for Sandbox {
    fn inspect(&self, name: &str) -> Result<Option<Container>> {
        self.runtime.inspect(name)
    }
    fn run(&self, request: &RunRequest) -> Result<Container> {
        let container = self.runtime.run(request)?;
        self.ids.borrow_mut().push(container.id.clone());
        Ok(container)
    }
    fn start(&self, id: &str) -> Result<()> {
        self.runtime.start(id)
    }
    fn exec(&self, id: &str, request: &ExecRequest) -> Result<i32> {
        self.runtime.exec(id, request)
    }
    fn stop(&self, id: &str) -> Result<()> {
        self.runtime.stop(id)
    }
    fn remove(&self, id: &str, force: bool) -> Result<()> {
        self.runtime.remove(id, force)
    }
    fn list_running(&self) -> Result<Vec<Container>> {
        self.runtime.list_running()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        for id in self.ids.get_mut() {
            let _ = self.runtime.remove(id, true);
        }
    }
}

#[test]
#[ignore = "requires Docker CLI and daemon"]
fn lifecycle_identity_workdir_environment_and_streams() {
    let sandbox = Sandbox::new(false);
    let uid = sandbox.host.uid.to_string();
    let gid = sandbox.host.gid.to_string();
    let cwd = sandbox.host.cwd.to_str().unwrap();
    assert_eq!(sandbox.execute(&[
        "sh", "-c",
        "test \"$(id -u)\" = \"$1\" && test \"$(id -g)\" = \"$2\" && test \"$PWD\" = \"$3\" && printf '%s' \"$ZENV_LIVE_VALUE\" > result",
        "check", &uid, &gid, cwd,
    ]), 0);
    let result = sandbox.host.cwd.join("result");
    assert_eq!(fs::read_to_string(&result).unwrap(), "one\ntwo");
    assert_eq!(fs::metadata(&result).unwrap().uid(), sandbox.host.uid);
    let container = sandbox.inspect(&sandbox.config.main.name).unwrap().unwrap();
    app::stop(&sandbox, &sandbox.config.main.name).unwrap();
    assert_eq!(
        sandbox
            .inspect(&sandbox.config.main.name)
            .unwrap()
            .unwrap()
            .state,
        ContainerState::Exited
    );
    assert_eq!(sandbox.execute(&["true"]), 0);
    assert_eq!(
        sandbox
            .inspect(&sandbox.config.main.name)
            .unwrap()
            .unwrap()
            .id,
        container.id
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_zenv"))
        .current_dir(&sandbox.host.cwd)
        .args(["exec", "sh", "-c", "cat; printf error >&2; exit 23"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"through stdin\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(23));
    assert_eq!(output.stdout, b"through stdin\n");
    assert_eq!(output.stderr, b"error");

    app::remove(&sandbox, &sandbox.config.main.name).unwrap();
    assert!(
        sandbox
            .inspect(&sandbox.config.main.name)
            .unwrap()
            .is_none()
    );
    // All host files were created inside the disposable project.
    assert!(sandbox.directory.path().join("Zenvfile").exists());
}

#[test]
#[ignore = "requires Docker CLI and daemon"]
fn initialization_failure_removes_created_container() {
    let sandbox = Sandbox::new(true);
    let error =
        app::execute(&sandbox, &sandbox.config, &["true".into()], &sandbox.host).unwrap_err();
    assert!(error.to_string().contains("17"));
    assert!(
        sandbox
            .inspect(&sandbox.config.main.name)
            .unwrap()
            .is_none()
    );
}

fn wait_until(child: &mut Child, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("PTY process exited before readiness: {status}");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("timed out waiting for PTY process");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires Docker CLI, daemon, and the script PTY utility"]
fn tty_and_ctrl_c() {
    let sandbox = Sandbox::new(false);
    assert_eq!(sandbox.execute(&["true"]), 0);
    let script = "test -t 0 && test -t 1 || exit 9; trap 'echo interrupted > interrupted; exit 130' INT; echo ready > ready; while :; do sleep 1; done";
    let binary = env!("CARGO_BIN_EXE_zenv");
    let mut command = Command::new("script");
    if cfg!(target_os = "macos") {
        command.args(["-q", "/dev/null", binary, "exec", "sh", "-c", script]);
    } else {
        let quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
        command.args([
            "-q",
            "-e",
            "-c",
            &format!("{} exec sh -c {}", quote(binary), quote(script)),
            "/dev/null",
        ]);
    }
    let mut child = command
        .current_dir(&sandbox.host.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let ready = sandbox.host.cwd.join("ready");
    wait_until(&mut child, || ready.exists());
    child.stdin.as_mut().unwrap().write_all(&[3]).unwrap();
    let interrupted = sandbox.host.cwd.join("interrupted");
    wait_until(&mut child, || interrupted.exists());
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("PTY process did not exit after Ctrl+C");
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        fs::read_to_string(interrupted).unwrap().trim(),
        "interrupted"
    );
}
