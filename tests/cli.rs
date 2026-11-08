#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    fs,
    io::{Seek, SeekFrom, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output, Stdio},
};

use tempfile::TempDir;

fn executable(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_zenv"));
    command.current_dir(directory);
    command
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn write_config(directory: &Path) {
    fs::write(
        directory.join("Zenvfile"),
        r#"
[main]
image = "example:latest"
name = "zenv-test"
[run]
init_commands = []
check_zenvfile_changes = false
[exec]
env_excludes = ["SECRET"]
[exec.options]
env = ["FROM_CONFIG=config"]
[aliases]
hello.command = ["printf", "hello"]
hello.description = "Say hello"
_hidden.command = ["private"]
"#,
    )
    .unwrap();
}

fn fake_docker(directory: &Path) -> TempDir {
    let binaries = tempfile::tempdir_in(directory).unwrap();
    executable(
        &binaries.path().join("docker"),
        r#"#!/bin/sh
case "$1" in
    container|ps)
        if [ "$2" = inspect ]; then
            if [ -n "$FAKE_FINGERPRINT" ]; then
                printf '{"zenv.config-hash":"%s"}\n' "$FAKE_FINGERPRINT"
            else
                printf '{}\n'
            fi
        else
            printf '%s\n' '{"ID":"existing-id","Names":"zenv-test","State":"running"}'
        fi
        ;;
    exec)
        printf '%s\000' "$@" > "$FAKE_LOG"
        if [ "$FAKE_READ_STDIN" = 1 ]; then cat; fi
        exit 37
        ;;
    stop|rm)
        if [ -n "$FAKE_ACTION_LOG" ]; then
            printf '%s\000' "$@" >> "$FAKE_ACTION_LOG"
        fi
        ;;
    *)
        printf 'unexpected Docker operation: %s\n' "$1" >&2
        exit 99
        ;;
esac
"#,
    );
    binaries
}

fn with_fake_path(command: &mut Command, binaries: &Path) {
    let mut paths = vec![binaries.to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    command.env("PATH", std::env::join_paths(paths).unwrap());
}

fn logged_args(path: &Path) -> Vec<Vec<u8>> {
    let bytes = fs::read(path).unwrap();
    bytes
        .strip_suffix(&[0])
        .unwrap()
        .split(|byte| *byte == 0)
        .map(<[u8]>::to_vec)
        .collect()
}

#[test]
fn initializes_with_flags_and_never_overwrites_an_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    let output = command(directory.path())
        .args(["init", "-i", "alpine:latest", "-c", "custom"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let config = fs::read_to_string(directory.path().join("Zenvfile")).unwrap();
    let parsed: toml::Value = toml::from_str(&config).unwrap();
    assert_eq!(parsed["main"]["image"].as_str(), Some("alpine:latest"));
    assert_eq!(parsed["main"]["name"].as_str(), Some("zenv-custom"));
    let output = command(directory.path()).arg("init").output().unwrap();
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(directory.path().join("Zenvfile")).unwrap(),
        config
    );
}

#[test]
fn initializes_from_a_custom_template() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("custom.toml"), "[main]\nimage = \"{image}\"\nname = \"{id}-{container_name}\"\n[exec]\nenv_excludes = []\n").unwrap();
    let output = command(directory.path())
        .args([
            "init",
            "--template",
            "custom.toml",
            "--image",
            "custom:tag",
            "--container-name",
            "chosen",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(directory.path().join("Zenvfile")).unwrap()).unwrap();
    assert_eq!(config["main"]["image"].as_str(), Some("custom:tag"));
    assert_eq!(config["main"]["name"].as_str(), Some("zenv-chosen"));
    assert!(config["exec"]["env_excludes"].is_array());
}

#[test]
fn reports_configuration_errors_without_panicking() {
    let directory = tempfile::tempdir().unwrap();
    let output = command(directory.path())
        .args(["info", "--zenvfile", "missing.toml"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("missing.toml"),
        "{}",
        stderr(&output)
    );
    fs::write(directory.path().join("Zenvfile"), "[main\n").unwrap();
    let output = command(directory.path()).arg("info").output().unwrap();
    assert!(!output.status.success());
    assert!(!stderr(&output).contains("panicked"));
}

#[test]
fn info_lists_public_aliases_and_exact_container_status() {
    let directory = tempfile::tempdir().unwrap();
    write_config(directory.path());
    let path = directory.path().join("Zenvfile");
    let explicit_description = "description".repeat(10);
    fs::write(
        &path,
        format!(
            "{}\nplain.command = ['echo', 'plain arg']\n\
         long.command = ['echo', '{}']\n\
         described.command = ['true']\n\
         described.description = '{explicit_description}'\n",
            fs::read_to_string(&path).unwrap(),
            "界".repeat(85),
        ),
    )
    .unwrap();
    let binaries = fake_docker(directory.path());
    let mut cmd = command(directory.path());
    with_fake_path(&mut cmd, binaries.path());
    let output = cmd.arg("info").output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Image: example:latest"), "{stdout}");
    assert!(stdout.contains("Container: zenv-test"), "{stdout}");
    assert!(stdout.contains("Status: Running"), "{stdout}");
    assert!(stdout.contains("hello: Say hello"), "{stdout}");
    assert!(stdout.contains(" - plain: echo plain arg\n"), "{stdout}");
    assert!(
        stdout.contains(&format!(" - long: echo {} ...\n", "界".repeat(69))),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!(" - described: {explicit_description}\n")),
        "{stdout}"
    );
    assert!(!stdout.contains("_hidden"), "{stdout}");
}

#[test]
fn exec_forwards_arguments_environment_and_exit_code() {
    let directory = tempfile::tempdir().unwrap();
    write_config(directory.path());
    let binaries = fake_docker(directory.path());
    let log = directory.path().join("argv");
    let mut cmd = command(directory.path());
    with_fake_path(&mut cmd, binaries.path());
    let output = cmd
        .env("FAKE_LOG", &log)
        .env("HOST_MARKER", "value with spaces\nand newline")
        .env("SECRET", "hidden")
        .args(["exec", "echo", "a b", "", "--help", "$(touch dangerous)"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(37), "{}", stderr(&output));
    let args = logged_args(&log);
    let expected = [
        "existing-id",
        "echo",
        "a b",
        "",
        "--help",
        "$(touch dangerous)",
    ]
    .map(|value| value.as_bytes().to_vec());
    assert!(args.ends_with(&expected), "{args:?}");
    assert!(args.contains(&b"HOST_MARKER=value with spaces\nand newline".to_vec()));
    assert!(args.contains(&b"FROM_CONFIG=config".to_vec()));
    assert!(!args.iter().any(|arg| arg.starts_with(b"SECRET=")));
    assert!(!directory.path().join("dangerous").exists());
}

#[test]
fn sh_entrypoint_preserves_arguments_and_exit_code() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("argv");
    executable(
        &directory.path().join("zenv"),
        "#!/bin/sh\nprintf '%s\\000' \"$@\" > \"$FAKE_LOG\"\nexit 53\n",
    );
    let wrapper = Path::new(env!("CARGO_MANIFEST_DIR")).join("bin/ze");
    let mut cmd = Command::new("/bin/sh");
    with_fake_path(&mut cmd, directory.path());
    let output = cmd
        .arg(wrapper)
        .args(["echo", "a b", "", "--help", "$literal"])
        .env("FAKE_LOG", &log)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(53), "{}", stderr(&output));
    assert_eq!(
        logged_args(&log),
        ["exec", "echo", "a b", "", "--help", "$literal"].map(|value| value.as_bytes().to_vec())
    );
}

#[test]
fn exec_preserves_non_utf8_arguments() {
    use std::os::unix::ffi::OsStringExt;

    let directory = tempfile::tempdir().unwrap();
    write_config(directory.path());
    let binaries = fake_docker(directory.path());
    let log = directory.path().join("argv");
    let argument = OsString::from_vec(vec![b'a', 0xff, b'b']);
    let mut cmd = command(directory.path());
    with_fake_path(&mut cmd, binaries.path());
    let output = cmd
        .env("FAKE_LOG", &log)
        .args([OsString::from("exec"), OsString::from("echo"), argument])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(37), "{}", stderr(&output));
    assert!(logged_args(&log).ends_with(&[vec![b'a', 0xff, b'b']]));
}

fn enable_fingerprint_check(directory: &Path) {
    let path = directory.join("Zenvfile");
    let source = fs::read_to_string(&path).unwrap();
    fs::write(
        path,
        source.replace(
            "check_zenvfile_changes = false",
            "check_zenvfile_changes = true",
        ),
    )
    .unwrap();
}

#[test]
fn noninteractive_mismatch_does_not_read_input_or_execute() {
    for saved_hash in [None, Some("outdated")] {
        let directory = tempfile::tempdir().unwrap();
        write_config(directory.path());
        enable_fingerprint_check(directory.path());
        let binaries = fake_docker(directory.path());
        let log = directory.path().join("argv");
        let mut input = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(directory.path().join("input"))
            .unwrap();
        input.write_all(b"command input\n").unwrap();
        input.seek(SeekFrom::Start(0)).unwrap();
        let mut cmd = command(directory.path());
        with_fake_path(&mut cmd, binaries.path());
        if let Some(hash) = saved_hash {
            cmd.env("FAKE_FINGERPRINT", hash);
        } else {
            cmd.env_remove("FAKE_FINGERPRINT");
        }
        let output = cmd
            .env("FAKE_LOG", &log)
            .stdin(Stdio::from(input.try_clone().unwrap()))
            .args(["exec", "cat"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(stderr(&output).contains("Warning:"), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("check_zenvfile_changes = false"),
            "{}",
            stderr(&output)
        );
        assert!(output.stdout.is_empty());
        assert_eq!(input.stream_position().unwrap(), 0);
        assert!(!log.exists());
    }
}

#[test]
fn disabled_check_preserves_piped_input_and_exit_code() {
    let directory = tempfile::tempdir().unwrap();
    write_config(directory.path());
    let binaries = fake_docker(directory.path());
    let log = directory.path().join("argv");
    let mut cmd = command(directory.path());
    with_fake_path(&mut cmd, binaries.path());
    let mut child = cmd
        .env("FAKE_LOG", &log)
        .env("FAKE_READ_STDIN", "1")
        .args(["exec", "cat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"command input\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(37), "{}", stderr(&output));
    assert_eq!(output.stdout, b"command input\n");
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
    assert!(log.exists());
}

#[test]
fn info_and_removal_bypass_fingerprint_confirmation() {
    let directory = tempfile::tempdir().unwrap();
    write_config(directory.path());
    enable_fingerprint_check(directory.path());
    let binaries = fake_docker(directory.path());
    let actions = directory.path().join("actions");
    for operation in ["info", "rm"] {
        let mut cmd = command(directory.path());
        with_fake_path(&mut cmd, binaries.path());
        let output = cmd
            .env("FAKE_ACTION_LOG", &actions)
            .env("FAKE_FINGERPRINT", "outdated")
            .arg(operation)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(output.stderr.is_empty(), "{}", stderr(&output));
    }
    let args = logged_args(&actions);
    assert!(
        args.windows(3)
            .any(|args| args == [b"rm".to_vec(), b"--".to_vec(), b"existing-id".to_vec()])
    );
}
