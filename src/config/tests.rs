use super::loader::find_file;
use super::*;
use indexmap::IndexSet;
use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};
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

fn project(source: &str) -> (TempDir, HostContext) {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join(DEFAULT_FILENAME), source).unwrap();
    let host = host(directory.path());
    (directory, host)
}

fn config(source: &str) -> (TempDir, HostContext, Config) {
    let (directory, host) = project(source);
    let config = FileConfigLoader::new(&host).load(None).unwrap();
    (directory, host, config)
}

fn option_values(options: &crate::runtime::Options, name: &str) -> Vec<String> {
    options[name].iter().cloned().collect()
}

#[test]
fn defaults_and_partial_builtin_overrides() {
    let (_directory, _host, config) = config(
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
    let request = config.exec_request(&["ls".into()]).unwrap();
    assert_eq!(option_values(&request.options, "user"), ["123:456"]);
    assert_eq!(option_values(&request.options, "tty"), ["false"]);
    assert_eq!(config.run_request().command, ["sleep", "365d"]);
}

#[test]
fn empty_arrays_replace_defaults_and_omitted_fields_are_inherited() {
    let (_directory, _host, config) = config(
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
    assert!(config.run.options["volume"].is_empty());
    assert_eq!(option_values(&config.run.options, "detach"), ["true"]);
    assert_eq!(config.run_request().command, ["true"]);
    let request = config.exec_request(&["env".into()]).unwrap();
    assert_eq!(option_values(&request.options, "user"), ["42"]);
    assert_eq!(option_values(&request.options, "interactive"), ["true"]);
}

#[test]
fn explicit_builtin_alias_is_used_verbatim() {
    let (_directory, _host, config) = config(
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
    let request = &config.run.init_commands[1];
    assert_eq!(
        request.command,
        ["useradd", "-m", "-r", "-u", "123", "-g", "456", "zenv"]
    );
    assert_eq!(option_values(&request.options, "user"), ["0:0"]);
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
        let config = FileConfigLoader::new(&host).load(None).unwrap();
        assert_eq!(
            option_values(&config.exec.options, "tty"),
            [tty.to_string()]
        );
    }
    fs::write(
        directory.path().join("Zenvfile"),
        "[exec.options]\ntty = false",
    )
    .unwrap();
    assert_eq!(
        option_values(
            &FileConfigLoader::new(&host)
                .load(None)
                .unwrap()
                .exec
                .options,
            "tty"
        ),
        ["false"]
    );
}

#[test]
fn environment_options_accept_scalar_and_mixed_lists() {
    for (value, expected) in [
        ("'KEY=value'", vec!["KEY=value"]),
        ("['KEY=value']", vec!["KEY=value"]),
        ("[true, 42, 'KEY=value']", vec!["42", "KEY=value", "true"]),
    ] {
        let (_directory, host) = project(&format!("[exec.options]\nenv = {value}"));
        let config = FileConfigLoader::new(&host).load(None).unwrap();
        assert_eq!(config.exec.env, expected);
    }
}

#[test]
fn all_option_values_are_ordered_sets_of_strings_including_false_values() {
    let (_directory, _host, config) = config(
        r#"
[run]
init_commands = [["setup"]]
[run.options]
interactive = true
cpus = 0.5
user = 42
publish = []
[exec.options]
interactive = [true, false, true]
mixed = ["text", false, 2, 0.25]
until = 2026-11-08T12:30:00Z
[aliases.setup]
command = ["setup"]
[aliases.setup.exec.options]
interactive = [false, true]
mixed = [false, "override"]
"#,
    );
    assert_eq!(option_values(&config.run.options, "interactive"), ["true"]);
    assert_eq!(option_values(&config.run.options, "cpus"), ["0.5"]);
    assert_eq!(option_values(&config.run.options, "user"), ["42"]);
    assert!(config.run.options["publish"].is_empty());
    assert_eq!(
        option_values(&config.exec.options, "interactive"),
        ["true", "false"]
    );
    assert_eq!(
        option_values(&config.exec.options, "mixed"),
        ["text", "false", "2", "0.25"]
    );
    assert_eq!(
        option_values(&config.exec.options, "until"),
        ["2026-11-08T12:30:00Z"]
    );
    let alias = config.exec_request(&["setup".into()]).unwrap();
    assert_eq!(
        option_values(&alias.options, "interactive"),
        ["false", "true"]
    );
    assert_eq!(
        option_values(&alias.options, "mixed"),
        ["false", "override"]
    );
    assert_eq!(
        option_values(&config.run.init_commands[0].options, "interactive"),
        option_values(&alias.options, "interactive")
    );
}

#[test]
fn duplicate_flag_values_are_removed_after_interpolation_in_first_seen_order() {
    let (_directory, _host, config) = config(
        r#"
[run]
init_commands = [["setup"]]
[run.options]
network = ["first", "second", "first"]
value = [false, "false", 42, "42", 42.0, nan, nan, "NaN"]
[exec.options]
interactive = [true, false, true, "true", "false"]
user = ["{uid}", 123, "123", "other", "other"]
env = ["KEY=first", "KEY=last", "KEY=first"]
[aliases.setup]
command = ["setup"]
[aliases.setup.exec.options]
user = [42, "42", false, "false", 42]
"#,
    );
    assert_eq!(
        option_values(&config.run.options, "network"),
        ["first", "second"]
    );
    assert_eq!(config.manage_network(), ["first", "second"]);
    assert_eq!(
        option_values(&config.run.options, "value"),
        ["false", "42", "NaN"]
    );
    assert_eq!(
        option_values(&config.exec.options, "interactive"),
        ["true", "false"]
    );
    assert_eq!(
        option_values(&config.exec.options, "user"),
        ["123", "other"]
    );
    assert_eq!(config.exec.env, ["KEY=last"]);
    let alias = &config.aliases["setup"].exec;
    assert_eq!(option_values(&alias.options, "user"), ["42", "false"]);
    assert_eq!(
        option_values(&config.run.init_commands[0].options, "user"),
        option_values(&alias.options, "user")
    );
}

#[test]
fn nested_option_values_are_rejected_in_each_options_table() {
    for source in [
        "[run.options]\ninteractive = [[true]]",
        "[exec.options]\nuser = { nested = 'value' }",
        "[aliases.work]\ncommand = ['echo']\n[aliases.work.exec.options]\nuser = [[42]]",
    ] {
        let (_directory, host) = project(source);
        assert!(
            FileConfigLoader::new(&host)
                .load(None)
                .unwrap_err()
                .to_string()
                .contains("values must be scalars")
        );
    }
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
        let config = FileConfigLoader::new(&host)
            .load(Some(&directory.path().join("Zenvfile")))
            .unwrap();
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
        FileConfigLoader::new(&host).load(None).unwrap().directory,
        directory.path()
    );
    assert!(
        FileConfigLoader::new(&host)
            .load(Some(Path::new("../../Zenvfile")))
            .is_ok()
    );
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
            FileConfigLoader::new(&host(directory.path()))
                .load(None)
                .is_err(),
            "{source}"
        );
    }
}

#[test]
fn aliases_expand_once_append_arguments_and_override_settings() {
    let (_directory, _host, config) = config(
        r#"
[run]
command = ["first", "--startup"]
init_commands = [["first", "--setup"]]
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
        .exec_request(&["first".into(), "--extra".into()])
        .unwrap();
    assert_eq!(request.command, ["second", "--fixed", "--extra"]);
    assert_eq!(option_values(&request.options, "user"), ["42"]);
    assert_eq!(option_values(&request.options, "tty"), ["true"]);
    assert_eq!(config.run.command, ["second", "--fixed", "--startup"]);
    assert_eq!(
        config.run_request().command,
        ["second", "--fixed", "--startup"]
    );
    let init = &config.run.init_commands[0];
    assert_eq!(init.command, ["second", "--fixed", "--setup"]);
    assert!(!init.options.contains_key("user"));
    assert_eq!(option_values(&init.options, "tty"), ["true"]);
}

#[test]
fn environment_priority_exclusions_and_multiline_dotenv() {
    let (directory, mut host) = project(
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
    let config = FileConfigLoader::new(&host).load(None).unwrap();
    let request = config.exec_request(&["env".into()]).unwrap();
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
    let (directory, mut host) = project(
        r#"
[exec]
env_file = ".env"
env_excludes = ["KEY"]
[aliases.test]
command = ["env"]
[aliases.test.exec]
env_file = ""
env_excludes = []
"#,
    );
    fs::write(directory.path().join(".env"), "DOTENV=dotenv\n").unwrap();
    host.environment.insert("KEY".into(), "value".into());
    let config = FileConfigLoader::new(&host).load(None).unwrap();
    assert_eq!(
        config.exec_request(&["env".into()]).unwrap().env,
        ["DOTENV=dotenv"]
    );
    assert_eq!(
        config.exec_request(&["test".into()]).unwrap().env,
        ["KEY=value"]
    );
}

#[test]
fn bare_environment_entries_are_resolved_from_the_host_snapshot() {
    let (_directory, mut host) = project(
        "[exec]\nenv_excludes = ['KEY', 'EMPTY', 'BINARY']\n\
         [exec.options]\nenv = ['KEY', 'EMPTY', 'BINARY', 'MISSING']",
    );
    let binary = OsString::from_vec(vec![0xff]);
    host.environment = BTreeMap::from([
        ("KEY".into(), "before".into()),
        ("EMPTY".into(), "".into()),
        ("BINARY".into(), binary.clone()),
    ]);
    let config = FileConfigLoader::new(&host).load(None).unwrap();
    host.environment.insert("KEY".into(), "after".into());
    host.environment.insert("MISSING".into(), "later".into());
    let mut binary_row = OsString::from("BINARY=");
    binary_row.push(binary);
    assert_eq!(
        config.exec_request(&["env".into()]).unwrap().env,
        [
            binary_row,
            "EMPTY=".into(),
            "KEY=before".into(),
            "MISSING".into(),
        ]
    );
}

#[test]
fn env_file_paths_use_invocation_directory_and_alias_overrides() {
    let (directory, mut host) = project(
        r#"
[exec]
env_file = ".env"
[aliases.root]
command = ["env"]
[aliases.root.exec]
env_file = "../.env"
"#,
    );
    let child = directory.path().join("child");
    fs::create_dir(&child).unwrap();
    fs::write(directory.path().join(".env"), "KEY=root\n").unwrap();
    fs::write(child.join(".env"), "KEY=child\n").unwrap();
    host.cwd = child;
    let config = FileConfigLoader::new(&host).load(None).unwrap();
    assert_eq!(
        config.exec_request(&["env".into()]).unwrap().env,
        ["KEY=child"]
    );
    assert_eq!(
        config.exec_request(&["root".into()]).unwrap().env,
        ["KEY=root"]
    );
}

#[test]
fn malformed_env_file_errors_include_its_path() {
    let (directory, host) = project("[exec]\nenv_file = '.env'");
    let path = directory.path().join(".env");
    fs::write(&path, "KEY='unterminated").unwrap();
    let error = FileConfigLoader::new(&host).load(None).unwrap_err();
    assert!(error.to_string().contains(path.to_str().unwrap()));
}

#[test]
fn invalid_environment_settings_fail_during_load_including_unused_aliases() {
    for (source, expected) in [
        ("[exec]\nenv_file = 'missing.env'", "missing.env"),
        (
            "[aliases.unused]\ncommand = ['echo']\n[aliases.unused.exec]\nenv_file = 'missing.env'",
            "missing.env",
        ),
        (
            "[aliases.unused]\ncommand = ['echo']\n[aliases.unused.exec.options]\nenv = [['KEY=value']]",
            "values must be scalars",
        ),
        (
            "[exec.options]\nenv = ['=invalid']",
            "environment variable names must not be empty",
        ),
    ] {
        let (_directory, host) = project(source);
        let error = FileConfigLoader::new(&host).load(None).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn malformed_unused_alias_env_file_fails_during_load() {
    let (directory, host) = project(
        "[aliases.unused]\ncommand = ['echo']\n[aliases.unused.exec]\nenv_file = 'unused.env'",
    );
    let path = directory.path().join("unused.env");
    fs::write(&path, "KEY='unterminated").unwrap();
    let error = FileConfigLoader::new(&host).load(None).unwrap_err();
    assert!(error.to_string().contains(path.to_str().unwrap()));
}

#[test]
fn loaded_config_keeps_all_requests_after_host_and_input_files_change() {
    let (directory, mut host) = project(
        r#"
[run]
command = ["stay", "--run"]
init_commands = [["bootstrap", "--init"]]
[exec]
env_file = "base.env"
env_excludes = ["HIDDEN"]
[exec.options]
env = ["FROM_CONFIG=config"]
user = "42"
[aliases.stay]
command = ["sleep", "365d"]
[aliases.special]
command = ["echo", "{uid}"]
[aliases.special.exec]
env_file = "alias.env"
[aliases.special.exec.options]
env = ["FROM_ALIAS=config"]
[aliases.bootstrap]
command = ["setup"]
[aliases.bootstrap.exec]
env_file = "init.env"
[aliases.bootstrap.exec.options]
user = "0:0"
"#,
    );
    for (file, value) in [
        ("base.env", "base"),
        ("alias.env", "alias"),
        ("init.env", "init"),
    ] {
        fs::write(directory.path().join(file), format!("FROM_FILE={value}\n")).unwrap();
    }
    host.environment = BTreeMap::from([
        ("HOST".into(), "before".into()),
        ("HIDDEN".into(), "secret".into()),
    ]);
    let config = FileConfigLoader::new(&host).load(None).unwrap();

    let run_hash = config.run_hash();

    fs::write(
        directory.path().join(DEFAULT_FILENAME),
        "invalid changed config",
    )
    .unwrap();
    for file in ["base.env", "alias.env", "init.env"] {
        fs::write(directory.path().join(file), "FROM_FILE=changed\n").unwrap();
        fs::remove_file(directory.path().join(file)).unwrap();
    }
    host.environment.clear();
    host.environment.insert("HOST".into(), "after".into());
    host.uid = 999;
    drop(host);
    drop(directory);

    let default = config.exec_request(&["env".into()]).unwrap();
    assert_eq!(
        default.env,
        ["FROM_CONFIG=config", "FROM_FILE=base", "HOST=before"]
    );
    assert_eq!(option_values(&default.options, "user"), ["42"]);
    let alias = config
        .exec_request(&["special".into(), "tail".into()])
        .unwrap();
    assert_eq!(alias.command, ["echo", "123", "tail"]);
    assert_eq!(
        alias.env,
        ["FROM_ALIAS=config", "FROM_FILE=alias", "HOST=before"]
    );
    let init = &config.run.init_commands[0];
    assert_eq!(init.command, ["setup", "--init"]);
    assert_eq!(init.env, ["FROM_FILE=init", "HIDDEN=secret", "HOST=before"]);
    assert_eq!(option_values(&init.options, "user"), ["0:0"]);
    assert!(!init.options.contains_key("interactive"));
    assert_eq!(config.run_request().command, ["sleep", "365d", "--run"]);
    assert_eq!(config.run_hash(), run_hash);
}

#[test]
fn empty_exec_commands_are_rejected_without_host_context() {
    let (_directory, _host, config) = config("");
    assert!(config.exec_request(&[]).is_err());
    assert!(config.exec_request(&["".into()]).is_err());
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
    let config = FileConfigLoader::new(&host).load(None).unwrap();
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
    let path = FileConfigLoader::new(&host)
        .init("python:3", None, Some("example"))
        .unwrap();
    let source = fs::read_to_string(&path).unwrap();
    assert!(source.contains("{zenvfilepath}"));
    let config = FileConfigLoader::new(&host).load(None).unwrap();
    assert_eq!(config.main.name, "zenv-example");
    assert_eq!(config.main.image, "python:3");
    let document: ::toml::Value = source.parse().unwrap();
    assert_eq!(
        document["exec"]["env_excludes"][0].as_str(),
        Some("KEY\"quoted")
    );
    assert!(config.exec.env.is_empty());
    host.tty = true;
    assert_eq!(
        option_values(
            &FileConfigLoader::new(&host)
                .load(None)
                .unwrap()
                .exec
                .options,
            "tty"
        ),
        ["true"]
    );
    assert!(
        FileConfigLoader::new(&host)
            .init(DEFAULT_IMAGE, None, None)
            .is_err()
    );
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn loader_trait_initializes_and_loads_without_overwriting() {
    let directory = tempfile::tempdir().unwrap();
    let host = host(directory.path());
    let file_loader = FileConfigLoader::new(&host);
    let loader: &dyn ConfigLoader = &file_loader;
    let path = loader.init("python:3", None, Some("trait-loader")).unwrap();
    let source = fs::read_to_string(&path).unwrap();
    let config = loader.load(Some(&path)).unwrap();
    assert_eq!(config.main.image, "python:3");
    assert_eq!(config.main.name, "zenv-trait-loader");
    assert!(loader.init(DEFAULT_IMAGE, None, None).is_err());
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
    FileConfigLoader::new(&host)
        .init(
            "image\"quoted",
            Some(Path::new("template")),
            Some("name'quoted"),
        )
        .unwrap();
    let config = FileConfigLoader::new(&host).load(None).unwrap();
    assert_eq!(config.main.name, "name'quoted");
    assert_eq!(config.main.image, "image\"quoted");
    let document: ::toml::Value = fs::read_to_string(directory.path().join(DEFAULT_FILENAME))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(document["exec"]["env_excludes"][0].as_str(), Some("KEY"));
    assert!(config.exec.env.is_empty());
}

#[test]
fn init_requires_valid_toml_and_preserves_explicit_exclusions() {
    let directory = tempfile::tempdir().unwrap();
    let mut host = host(directory.path());
    host.environment.insert("HOST".into(), "value".into());
    let template = directory.path().join("template");
    fs::write(&template, "[exec]\nenv_excludes = {env_excludes}").unwrap();
    assert!(
        FileConfigLoader::new(&host)
            .init(DEFAULT_IMAGE, Some(&template), None)
            .is_err()
    );
    assert!(!directory.path().join(DEFAULT_FILENAME).exists());
    fs::write(&template, "[exec]\nenv_excludes = ['KEEP']").unwrap();
    host.environment.insert("KEEP".into(), "excluded".into());
    FileConfigLoader::new(&host)
        .init(DEFAULT_IMAGE, Some(&template), None)
        .unwrap();
    assert_eq!(
        FileConfigLoader::new(&host).load(None).unwrap().exec.env,
        ["HOST=value"]
    );
}

#[test]
fn non_utf8_arguments_and_environment_are_preserved() {
    let (_directory, mut host) = project("");
    let bytes = OsString::from_vec(vec![0xff, b'a']);
    host.environment.insert("KEY".into(), bytes.clone());
    let config = FileConfigLoader::new(&host).load(None).unwrap();
    let request = config
        .exec_request(&["echo".into(), bytes.clone()])
        .unwrap();
    assert_eq!(request.command[1], bytes);
    let mut row = OsString::from("KEY=");
    row.push(bytes);
    assert_eq!(request.env, [row]);
}

fn fingerprint_of(directory: &Path, source: &str, host: &HostContext) -> String {
    fs::write(directory.join(DEFAULT_FILENAME), source).unwrap();
    FileConfigLoader::new(host).load(None).unwrap().run_hash()
}

#[test]
fn run_policies_default_to_true_and_do_not_change_fingerprint() {
    let (directory, host, config) = config("");
    assert!(config.run.manage_network);
    assert!(config.run.check_zenvfile_changes);
    assert!(config.manage_network().is_empty());
    assert_eq!(
        config.run_request().fingerprint.as_deref(),
        Some(config.run_hash().as_str())
    );
    assert_eq!(config.run_hash().len(), 64);
    assert!(
        config
            .run_hash()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    );
    for flag in ["manage_network", "check_zenvfile_changes"] {
        let source = format!("[run]\n{flag} = false\n");
        assert_eq!(
            fingerprint_of(directory.path(), &source, &host),
            config.run_hash()
        );
        let loaded = FileConfigLoader::new(&host).load(None).unwrap();
        assert_eq!(loaded.run.manage_network, flag != "manage_network");
        assert_eq!(
            loaded.run.check_zenvfile_changes,
            flag != "check_zenvfile_changes"
        );
        for invalid in ["'false'", "0", "[]"] {
            fs::write(
                directory.path().join(DEFAULT_FILENAME),
                format!("[run]\n{flag} = {invalid}\n"),
            )
            .unwrap();
            let error = FileConfigLoader::new(&host).load(None).unwrap_err();
            assert!(error.to_string().contains(flag), "{error}");
        }
    }
}

#[test]
fn configured_networks_preserve_explicit_names_without_filtering() {
    for (value, expected) in [
        ("'project-dev'", vec!["project-dev"]),
        (
            "['project-dev', 'project-dev', 'bridge', 'name=dev,alias=app']",
            vec!["project-dev", "bridge", "name=dev,alias=app"],
        ),
        ("[]", vec![]),
        ("true", vec!["true"]),
        ("[true, 42, 'project']", vec!["true", "42", "project"]),
    ] {
        let (_directory, _host, mut config) = config(&format!("[run.options]\nnetwork = {value}"));
        assert_eq!(config.manage_network(), expected);
        assert!(config.run.options.contains_key("network"));
        config
            .run
            .options
            .insert("network".into(), IndexSet::from(["changed".into()]));
        assert_eq!(config.manage_network(), ["changed"]);
        config.run.manage_network = false;
        assert!(config.manage_network().is_empty());
    }
}

const FINGERPRINT_SETUP: &str = r#"
[main]
image = "example:latest"
[run]
command = ["stay"]
init_commands = [["setup", "--first"]]
[run.options]
publish = ["8000:8000"]
[aliases.stay]
command = ["sleep", "365d"]
[aliases.setup]
command = ["provision", "{uid}", "{gid}", "{pwd}", "{tty}"]
description = "Initialize"
[aliases.setup.exec.options]
user = "0:0"
env = ["MODE=setup"]
[aliases.unused]
command = ["unused-command"]
"#;

#[test]
fn creation_declarations_change_the_fingerprint() {
    let (directory, host) = project(FINGERPRINT_SETUP);
    let previous = fingerprint_of(directory.path(), FINGERPRINT_SETUP, &host);
    for (before, after) in [
        ("example:latest", "example:new"),
        ("8000:8000", "9000:9000"),
        ("[run.options]", "[run.options]\nnetwork = 'another'"),
        (
            "[run.options]",
            "[run.options]\nvolume = ['/other:/project']",
        ),
        ("[run.options]", "[run.options]\ndetach = false"),
        ("365d", "30d"),
        ("--first", "--changed"),
        ("provision", "different-provisioner"),
        ("0:0", "123:456"),
        ("MODE=setup", "MODE=changed"),
    ] {
        let source = FINGERPRINT_SETUP.replace(before, after);
        assert_ne!(
            fingerprint_of(directory.path(), &source, &host),
            previous,
            "{before}"
        );
    }
}

#[test]
fn unrelated_configuration_does_not_change_the_fingerprint() {
    let (directory, host) = project(FINGERPRINT_SETUP);
    let previous = fingerprint_of(directory.path(), FINGERPRINT_SETUP, &host);
    for source in [
        FINGERPRINT_SETUP.replace("[main]", "[main]\nname = 'other'\ndebug = true"),
        FINGERPRINT_SETUP.replace("unused-command", "other-unused-command"),
        FINGERPRINT_SETUP.replace("Initialize", "New description"),
        format!(
            "{FINGERPRINT_SETUP}\n[exec]\nenv_excludes = ['KEY']\n[exec.options]\nuser = '42'\nenv = ['KEY=new']"
        ),
        format!("{FINGERPRINT_SETUP}\n[aliases.stay.exec.options]\nuser = '123'"),
    ] {
        assert_eq!(fingerprint_of(directory.path(), &source, &host), previous);
    }
}

#[test]
fn comments_key_order_and_explicit_defaults_have_the_same_fingerprint() {
    let (directory, host, config) = config("");
    let source = r#"
# Equivalent settings in another order.
[run.options]
publish = []
detach = true
volume = ["{zenvfilepath}:{zenvfilepath}:rw"]
[run]
init_commands = [["__create_group__"], ["__create_user__"]]
command = ["__sleep__"]
manage_network = true
check_zenvfile_changes = true
[main]
image = "ubuntu:latest"
debug = false
version = 1
"#;
    assert_eq!(
        fingerprint_of(directory.path(), source, &host),
        config.run_hash()
    );
}

#[test]
fn omitted_and_explicit_empty_init_exec_settings_have_the_same_fingerprint() {
    let source = "[run]\ninit_commands = [['setup']]\n[aliases.setup]\ncommand = ['provision']\n";
    let (directory, mut host) = project(source);
    host.environment.insert("KEY".into(), "value".into());
    let baseline = FileConfigLoader::new(&host).load(None).unwrap();
    for suffix in [
        "[aliases.setup.exec]\n",
        "[aliases.setup.exec]\nenv_file = ''\n",
        "[aliases.setup.exec]\nenv_excludes = []\n",
        "[aliases.setup.exec]\nenv_excludes = ['ABSENT']\n",
        "[aliases.setup.exec.options]\n",
        "[aliases.setup.exec]\nenv_file = ''\nenv_excludes = []\noptions = {}\n",
    ] {
        fs::write(
            directory.path().join(DEFAULT_FILENAME),
            format!("{source}{suffix}"),
        )
        .unwrap();
        let loaded = FileConfigLoader::new(&host).load(None).unwrap();
        assert_eq!(loaded.run_hash(), baseline.run_hash(), "{suffix}");
        let request = &loaded.run.init_commands[0];
        assert_eq!(request.command, baseline.run.init_commands[0].command);
        assert_eq!(request.env, baseline.run.init_commands[0].env);
        assert!(request.options.is_empty());
    }
    for suffix in [
        "[aliases.setup.exec]\nenv_excludes = ['KEY']\n",
        "[aliases.setup.exec.options]\nuser = '42'\n",
    ] {
        assert_ne!(
            fingerprint_of(directory.path(), &format!("{source}{suffix}"), &host),
            baseline.run_hash(),
            "{suffix}"
        );
    }
}

#[test]
fn scalar_singleton_and_equivalent_flag_values_have_the_same_fingerprint() {
    let (directory, host) = project("");
    for (scalar, list, text) in [
        ("true", "[true]", "['true']"),
        ("false", "[false]", "['false']"),
        ("42", "[42]", "['42']"),
        ("0.5", "[0.5]", "['0.5']"),
        ("nan", "[nan]", "['NaN']"),
        ("inf", "[inf]", "['inf']"),
    ] {
        let source = |value| {
            format!(
                "[run]\ninit_commands = [['setup']]\n[run.options]\nvalue = {value}\n\
             [aliases.setup]\ncommand = ['setup']\n[aliases.setup.exec.options]\nvalue = {value}"
            )
        };
        let previous = fingerprint_of(directory.path(), &source(scalar), &host);
        assert_eq!(
            fingerprint_of(directory.path(), &source(list), &host),
            previous
        );
        assert_eq!(
            fingerprint_of(directory.path(), &source(text), &host),
            previous
        );
        assert_eq!(
            fingerprint_of(
                directory.path(),
                &source(&format!("[{scalar}, {scalar}]")),
                &host,
            ),
            previous
        );
    }
    let source = |value| {
        format!(
            "[run]\ninit_commands = [['setup']]\n[run.options]\nvalue = {value}\n\
         [aliases.setup]\ncommand = ['setup']\n[aliases.setup.exec.options]\nvalue = {value}"
        )
    };
    assert_eq!(
        fingerprint_of(directory.path(), &source("[true, false]"), &host),
        fingerprint_of(
            directory.path(),
            &source("[true, false, true, 'false']"),
            &host
        )
    );
    assert_ne!(
        fingerprint_of(
            directory.path(),
            "[run.options]\ninteractive = [true, false]",
            &host
        ),
        fingerprint_of(
            directory.path(),
            "[run.options]\ninteractive = [false, true]",
            &host
        ),
    );
}

#[test]
fn resolved_invocation_values_change_the_run_hash_when_used_by_init() {
    let (directory, host, config) = config(FINGERPRINT_SETUP);
    let child = directory.path().join("nested");
    fs::create_dir(&child).unwrap();
    for change in ["cwd", "uid", "gid", "tty", "env"] {
        let mut changed = host.clone();
        match change {
            "cwd" => changed.cwd = child.clone(),
            "uid" => changed.uid = 987,
            "gid" => changed.gid = 654,
            "tty" => changed.tty = true,
            "env" => {
                changed.environment.insert("CHANGED".into(), "host".into());
            }
            _ => unreachable!(),
        }
        let loaded = FileConfigLoader::new(&changed).load(None).unwrap();
        assert_ne!(loaded.run_hash(), config.run_hash(), "{change}");
    }
    let (_other_directory, _other_host, other) = self::config(FINGERPRINT_SETUP);
    assert_ne!(other.run_hash(), config.run_hash());
}

#[test]
fn host_changes_do_not_change_the_run_hash_without_affected_creation_settings() {
    let (directory, mut host, baseline) = config("[run]\ninit_commands = []\n");
    let child = directory.path().join("nested");
    fs::create_dir(&child).unwrap();
    host.cwd = child;
    host.uid = 987;
    host.gid = 654;
    host.tty = true;
    host.environment.insert("CHANGED".into(), "host".into());
    let loaded = FileConfigLoader::new(&host).load(None).unwrap();
    assert_eq!(loaded.run_hash(), baseline.run_hash());
    assert_ne!(loaded.exec.env, baseline.exec.env);
    assert_ne!(
        option_values(&loaded.exec.options, "user"),
        option_values(&baseline.exec.options, "user")
    );
}

#[test]
fn run_hash_reflects_current_creation_settings_and_ignores_execution_policy() {
    let (_directory, _host, baseline) = config(FINGERPRINT_SETUP);
    let previous = baseline.run_hash();
    for change in [
        "image",
        "directory",
        "command",
        "options",
        "init_command",
        "init_options",
        "init_env",
    ] {
        let mut changed = baseline.clone();
        match change {
            "image" => changed.main.image.push_str("-changed"),
            "directory" => changed.directory.push("changed"),
            "command" => changed.run.command.push("--changed".into()),
            "options" => {
                changed
                    .run
                    .options
                    .insert("network".into(), IndexSet::from(["changed".into()]));
            }
            "init_command" => changed.run.init_commands[0]
                .command
                .push(OsString::from_vec(vec![0xff])),
            "init_options" => {
                changed.run.init_commands[0]
                    .options
                    .insert("user".into(), IndexSet::from(["42".into()]));
            }
            "init_env" => changed.run.init_commands[0]
                .env
                .push(OsString::from_vec(vec![b'K', b'=', 0xff])),
            _ => unreachable!(),
        }
        assert_ne!(changed.run_hash(), previous, "{change}");
        let first = changed.run_hash();
        if change == "init_command" {
            *changed.run.init_commands[0].command.last_mut().unwrap() =
                OsString::from_vec(vec![0xfe]);
            assert_ne!(changed.run_hash(), first);
        }
        if change == "init_env" {
            *changed.run.init_commands[0].env.last_mut().unwrap() =
                OsString::from_vec(vec![b'K', b'=', 0xfe]);
            assert_ne!(changed.run_hash(), first);
        }
    }
    let mut changed = baseline;
    changed.main.name.push_str("-changed");
    changed.main.debug = !changed.main.debug;
    changed.run.manage_network = !changed.run.manage_network;
    changed.run.check_zenvfile_changes = !changed.run.check_zenvfile_changes;
    changed.exec.env.push("EXEC_ONLY=changed".into());
    changed
        .exec
        .options
        .insert("user".into(), IndexSet::from(["42".into()]));
    assert_eq!(changed.run_hash(), previous);
}

#[test]
fn active_and_escaped_init_placeholders_have_different_fingerprints() {
    let active =
        "[run]\ninit_commands = [['work']]\n[aliases]\nwork = { command = ['echo', '{uid}'] }";
    let (directory, host, config) = config(active);
    let literal = active.replace("'{uid}'", "'{{uid}}'");
    assert_ne!(
        fingerprint_of(directory.path(), &literal, &host),
        config.run_hash()
    );
    let loaded = FileConfigLoader::new(&host).load(None).unwrap();
    assert_eq!(loaded.run.init_commands[0].command, ["echo", "{uid}"]);
}

#[test]
fn equivalent_expanded_startup_commands_have_the_same_fingerprint() {
    let source = "[run]\ncommand = ['stay', '365d']\n[aliases.stay]\ncommand = ['sleep']";
    let (directory, host, config) = config(source);
    let expanded = source.replace("['stay', '365d']", "['sleep', '365d']");
    assert_eq!(
        fingerprint_of(directory.path(), &expanded, &host),
        config.run_hash()
    );
}

#[test]
fn init_env_file_contents_change_the_run_hash() {
    let source = "[run]\ninit_commands = [['setup']]\n[exec]\nenv_file = '.env'\n[aliases.setup]\ncommand = ['setup-command']\n[aliases.setup.exec]\nenv_file = '.env'";
    let (directory, host) = project(source);
    let path = directory.path().join(".env");
    fs::write(&path, "KEY=before\n").unwrap();
    let before = FileConfigLoader::new(&host).load(None).unwrap();
    fs::write(directory.path().join("same.env"), "KEY=before\n").unwrap();
    fs::write(
        directory.path().join(DEFAULT_FILENAME),
        source.replace("env_file = '.env'", "env_file = 'same.env'"),
    )
    .unwrap();
    let same = FileConfigLoader::new(&host).load(None).unwrap();
    assert_eq!(before.run_hash(), same.run_hash());
    fs::write(directory.path().join(DEFAULT_FILENAME), source).unwrap();
    fs::write(path, "KEY=after\n").unwrap();
    let after = FileConfigLoader::new(&host).load(None).unwrap();
    assert_ne!(before.run_hash(), after.run_hash());
    assert_ne!(before.exec.env, after.exec.env);
}

#[test]
fn ordinary_exec_env_file_contents_do_not_change_the_run_hash() {
    let source = "[run]\ninit_commands = []\n[exec]\nenv_file = '.env'\n";
    let (directory, host) = project(source);
    let path = directory.path().join(".env");
    fs::write(&path, "KEY=before\n").unwrap();
    let before = FileConfigLoader::new(&host).load(None).unwrap();
    fs::write(path, "KEY=after\n").unwrap();
    let after = FileConfigLoader::new(&host).load(None).unwrap();
    assert_eq!(before.run_hash(), after.run_hash());
    assert_ne!(before.exec.env, after.exec.env);
}
