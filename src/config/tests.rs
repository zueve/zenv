use super::*;
use crate::runtime::OptionValue;
use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
use tempfile::TempDir;

fn host(path: &Path) -> HostContext {
    HostContext {
        cwd: path.into(),
        uid: 123,
        gid: 456,
        tty: false,
        environment: BTreeMap::new(),
    }
}

fn config(source: &str) -> (TempDir, HostContext, Config) {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join(DEFAULT_FILENAME), source).unwrap();
    let host = host(directory.path());
    let config = Config::load(None, &host).unwrap();
    (directory, host, config)
}

#[test]
fn defaults_and_partial_builtin_overrides() {
    let (_directory, host, config) = config(
        r#"
[aliases.__sleep__]
description = "Keep running"
[run.options]
publish = ["8000:8000"]
"#,
    );
    assert_eq!(config.main.image, DEFAULT_IMAGE);
    assert_eq!(config.main.version, 1);
    assert_eq!(config.aliases["__sleep__"].command, ["sleep", "365d"]);
    assert_eq!(
        config.aliases["__sleep__"].description.as_deref(),
        Some("Keep running")
    );
    assert_eq!(config.run.init_commands.len(), 2);
    let request = config.prepare_exec(&["ls".into()], &host, true).unwrap();
    assert_eq!(request.options["user"], OptionValue::Text("123:456".into()));
    assert_eq!(request.options["tty"], OptionValue::Flag(false));
    assert_eq!(config.prepare_run().unwrap().command, ["sleep", "365d"]);
}

#[test]
fn empty_arrays_replace_defaults_and_omitted_fields_are_inherited() {
    let (_directory, host, config) = config(
        r#"
[run]
init_commands = []
[run.options]
volume = []
[exec.options]
user = "42"
[aliases.__sleep__]
command = ["true"]
"#,
    );
    assert!(config.run.init_commands.is_empty());
    assert_eq!(config.run.options["volume"], OptionValue::List(vec![]));
    assert_eq!(config.run.options["detach"], OptionValue::Flag(true));
    assert_eq!(config.prepare_run().unwrap().command, ["true"]);
    let request = config.prepare_exec(&["env".into()], &host, true).unwrap();
    assert_eq!(request.options["user"], OptionValue::Text("42".into()));
    assert_eq!(request.options["interactive"], OptionValue::Flag(true));
}

#[test]
fn explicit_builtin_alias_is_used_verbatim() {
    let (_directory, host, config) = config(
        r#"
[aliases.__create_user__]
command = ["useradd", "-m", "-r", "-u", "{uid}", "-g", "{gid}", "zenv"]
description = "Project user"
[aliases.__create_user__.exec.options]
user = "0:0"
"#,
    );
    let alias = &config.aliases["__create_user__"];
    assert_eq!(alias.description.as_deref(), Some("Project user"));
    let request = config
        .prepare_exec(&["__create_user__".into()], &host, false)
        .unwrap();
    assert_eq!(
        request.command,
        ["useradd", "-m", "-r", "-u", "123", "-g", "456", "zenv"]
    );
    assert_eq!(request.options["user"], OptionValue::Text("0:0".into()));
}

#[test]
fn merging_defaults_does_not_expand_init_only_or_escaped_placeholders() {
    let (_directory, _host, config) = config(
        r#"
[aliases.show]
command = ["{image}", "{id}", "{container_name}", "{{uid}}", "{uid}"]
"#,
    );
    assert_eq!(
        config.aliases["show"].command,
        ["{image}", "{id}", "{container_name}", "{uid}", "123"]
    );
}

#[test]
fn tty_tracks_stdin_and_can_be_overridden() {
    let (directory, mut host, _) = config("");
    for tty in [false, true] {
        host.tty = tty;
        let config = Config::load(None, &host).unwrap();
        assert_eq!(config.exec.options["tty"], OptionValue::Flag(tty));
    }
    fs::write(
        directory.path().join("Zenvfile"),
        "[exec.options]\ntty = false",
    )
    .unwrap();
    assert_eq!(
        Config::load(None, &host).unwrap().exec.options["tty"],
        OptionValue::Flag(false)
    );
}

#[test]
fn environment_options_require_an_array() {
    let (_directory, host, config) = config("[exec.options]\nenv = 'KEY=value'");
    let error = config
        .prepare_exec(&["env".into()], &host, true)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("exec.options.env must be an array of strings")
    );
}

fn account_tools() -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    for (name, body) in [
        (
            "getent",
            "case \"$ACCOUNT_CASE:$2\" in existing:*) exit 0;; occupied:zenv) exit 0;; *) exit 2;; esac",
        ),
        ("groupadd", "printf '%s\\n' \"$@\""),
        ("useradd", "printf '%s\\n' \"$@\""),
    ] {
        let path = directory.path().join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    directory
}

fn setup_output(config: &Config, alias: &str, tools: &Path, case: &str) -> String {
    let command = &config.aliases[alias].command;
    assert_eq!(command[0], "sh");
    let output = std::process::Command::new("/bin/sh")
        .args(&command[1..])
        .env("PATH", tools)
        .env("ACCOUNT_CASE", case)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn default_setup_reuses_root_and_existing_identities() {
    let tools = account_tools();
    let (directory, mut host, _) = config("");
    for (uid, gid) in [(0, 0), (1000, 1000)] {
        host.uid = uid;
        host.gid = gid;
        let config = Config::load(Some(&directory.path().join("Zenvfile")), &host).unwrap();
        for alias in ["__create_group__", "__create_user__"] {
            assert_eq!(setup_output(&config, alias, tools.path(), "existing"), "");
        }
    }
}

#[test]
fn builtin_setup_creates_missing_accounts_and_handles_occupied_names() {
    let tools = account_tools();
    let (_directory, _host, config) = config("");
    for (case, group, user) in [
        ("missing", "zenv", "zenv"),
        ("occupied", "zenv-456", "zenv-123"),
    ] {
        assert_eq!(
            setup_output(&config, "__create_group__", tools.path(), case),
            format!("-o\n--gid\n456\n{group}\n")
        );
        assert_eq!(
            setup_output(&config, "__create_user__", tools.path(), case),
            format!("-m\n-r\n-u\n123\n-g\n456\n{user}\n")
        );
    }
}

#[test]
fn custom_account_setup_is_not_rewritten() {
    let (_directory, _host, config) =
        config("[aliases]\n__create_user__.command = ['useradd', '--custom']");
    assert_eq!(
        config.aliases["__create_user__"].command,
        ["useradd", "--custom"]
    );
}

#[test]
fn finds_ancestors_and_resolves_relative_explicit_paths() {
    let (directory, mut host, _) = config("");
    let child = directory.path().join("a/b");
    fs::create_dir_all(&child).unwrap();
    host.cwd = child;
    assert_eq!(
        Config::load(None, &host).unwrap().directory,
        directory.path()
    );
    assert!(Config::load(Some(Path::new("../../Zenvfile")), &host).is_ok());
}

#[test]
fn root_search_terminates() {
    let result = find_file(Path::new("/"));
    assert_eq!(
        result,
        Path::new("/Zenvfile")
            .is_file()
            .then(|| PathBuf::from("/Zenvfile"))
    );
}

#[test]
fn invalid_toml_version_and_empty_commands_are_errors() {
    for source in [
        "[",
        "[main]\nversion = 2",
        "[run]\ncommand = []",
        "[aliases.bad]\ndescription = 'missing command'",
    ] {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(DEFAULT_FILENAME), source).unwrap();
        assert!(
            Config::load(None, &host(directory.path())).is_err(),
            "{source}"
        );
    }
}

#[test]
fn aliases_expand_once_append_arguments_and_override_settings() {
    let (_directory, host, config) = config(
        r#"
[exec.options]
user = "42"
[aliases.first]
command = ["second", "--fixed"]
[aliases.first.exec.options]
tty = true
[aliases.second]
command = ["echo"]
"#,
    );
    let request = config
        .prepare_exec(&["first".into(), "--extra".into()], &host, true)
        .unwrap();
    assert_eq!(request.command, ["second", "--fixed", "--extra"]);
    assert_eq!(request.options["user"], OptionValue::Text("42".into()));
    assert_eq!(request.options["tty"], OptionValue::Flag(true));
    let init = config
        .prepare_exec(&["first".into()], &host, false)
        .unwrap();
    assert!(!init.options.contains_key("user"));
}

#[test]
fn environment_priority_exclusions_and_multiline_dotenv() {
    let (directory, mut host, config) = config(
        r#"
[exec]
env_file = ".env"
env_excludes = ["EXCLUDED"]
[exec.options]
env = ["KEY=config", "CONFIG=config", "EXCLUDED=config"]
"#,
    );
    fs::write(
        directory.path().join(".env"),
        "KEY=dotenv\nCONFIG=dotenv\nDOTENV=dotenv\nMULTI=\"one\ntwo\"\n",
    )
    .unwrap();
    host.environment = BTreeMap::from([
        ("KEY".into(), "host".into()),
        ("EXCLUDED".into(), "host".into()),
    ]);
    let request = config.prepare_exec(&["env".into()], &host, true).unwrap();
    assert_eq!(
        request.env,
        [
            "CONFIG=config",
            "DOTENV=dotenv",
            "EXCLUDED=config",
            "KEY=host",
            "MULTI=one\ntwo"
        ]
    );
    assert!(!request.options.contains_key("env"));
}

#[test]
fn alias_can_clear_env_file_and_replace_exclusions() {
    let (_directory, mut host, config) = config(
        r#"
[exec]
env_file = "missing.env"
env_excludes = ["KEY"]
[aliases.test]
command = ["env"]
[aliases.test.exec]
env_file = ""
env_excludes = []
"#,
    );
    host.environment.insert("KEY".into(), "value".into());
    assert!(config.prepare_exec(&["env".into()], &host, true).is_err());
    assert_eq!(
        config
            .prepare_exec(&["test".into()], &host, true)
            .unwrap()
            .env,
        ["KEY=value"]
    );
}

#[test]
fn interpolation_handles_quoted_paths_and_unknown_tokens() {
    let (_directory, mut host, _) = config("");
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("with\"quote\\slash");
    fs::create_dir(&path).unwrap();
    fs::write(
        path.join(DEFAULT_FILENAME),
        "[aliases.show]\ncommand = ['{pwd}', '{unknown}', '{{uid}}', '{uid}']",
    )
    .unwrap();
    host.cwd = path.clone();
    let config = Config::load(None, &host).unwrap();
    assert_eq!(
        config.aliases["show"].command,
        [path.to_str().unwrap(), "{unknown}", "{uid}", "123"]
    );
}

#[test]
fn init_preserves_runtime_placeholders_and_does_not_overwrite() {
    let directory = tempfile::tempdir().unwrap();
    let mut host = host(directory.path());
    host.environment
        .insert("KEY\"quoted".into(), "value".into());
    let path = init(&host, "python:3", None, Some("example")).unwrap();
    let source = fs::read_to_string(&path).unwrap();
    assert!(source.contains("{zenvfilepath}"));
    let config = Config::load(None, &host).unwrap();
    assert_eq!(config.main.name, "zenv-example");
    assert_eq!(config.main.image, "python:3");
    assert_eq!(config.exec.env_excludes, Some(vec!["KEY\"quoted".into()]));
    host.tty = true;
    assert_eq!(
        Config::load(None, &host).unwrap().exec.options["tty"],
        OptionValue::Flag(true)
    );
    assert!(init(&host, DEFAULT_IMAGE, None, None).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn custom_init_template_is_relative_and_escaped_safely() {
    let directory = tempfile::tempdir().unwrap();
    let mut host = host(directory.path());
    host.environment.insert("KEY".into(), "value".into());
    fs::write(
        directory.path().join("template"),
        "[main]\nimage = '{image}'\nname = '{container_name}'\n[exec]\nenv_excludes = []\n",
    )
    .unwrap();
    init(
        &host,
        "image\"quoted",
        Some(Path::new("template")),
        Some("name'quoted"),
    )
    .unwrap();
    let config = Config::load(None, &host).unwrap();
    assert_eq!(config.main.name, "name'quoted");
    assert_eq!(config.main.image, "image\"quoted");
    assert_eq!(config.exec.env_excludes, Some(vec!["KEY".into()]));
}

#[test]
fn init_requires_valid_toml_and_preserves_explicit_exclusions() {
    let directory = tempfile::tempdir().unwrap();
    let mut host = host(directory.path());
    host.environment.insert("HOST".into(), "value".into());
    let template = directory.path().join("template");
    fs::write(&template, "[exec]\nenv_excludes = {env_excludes}").unwrap();
    assert!(init(&host, DEFAULT_IMAGE, Some(&template), None).is_err());
    assert!(!directory.path().join(DEFAULT_FILENAME).exists());
    fs::write(&template, "[exec]\nenv_excludes = ['KEEP']").unwrap();
    init(&host, DEFAULT_IMAGE, Some(&template), None).unwrap();
    assert_eq!(
        Config::load(None, &host).unwrap().exec.env_excludes,
        Some(vec!["KEEP".into()])
    );
}

#[test]
fn non_utf8_arguments_and_environment_are_preserved() {
    let (_directory, mut host, config) = config("");
    let bytes = OsString::from_vec(vec![0xff, b'a']);
    host.environment.insert("KEY".into(), bytes.clone());
    let request = config
        .prepare_exec(&["echo".into(), bytes.clone()], &host, true)
        .unwrap();
    assert_eq!(request.command[1], bytes);
    let mut row = OsString::from("KEY=");
    row.push(bytes);
    assert_eq!(request.env, [row]);
}
