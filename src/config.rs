use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::IsTerminal,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{Error, Result, runtime::Options};

mod init;
pub use init::init;

pub const DEFAULT_FILENAME: &str = "Zenvfile";
pub const DEFAULT_IMAGE: &str = "ubuntu:latest";
const DEFAULTS: &str = include_str!("config/defaults.toml");

/// A snapshot allows configuration to be resolved without modifying global state.
#[derive(Clone, Debug)]
pub struct HostContext {
    pub cwd: PathBuf,
    pub uid: u32,
    pub gid: u32,
    pub tty: bool,
    pub environment: BTreeMap<OsString, OsString>,
}

impl HostContext {
    pub fn capture() -> Result<Self> {
        Ok(Self {
            cwd: std::env::current_dir().map_err(|error| Error::io(".", error))?,
            uid: rustix::process::getuid().as_raw(),
            gid: rustix::process::getgid().as_raw(),
            tty: std::io::stdin().is_terminal(),
            environment: std::env::vars_os().collect(),
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct MainConfig {
    pub image: String,
    pub name: String,
    pub debug: bool,
    pub version: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RunConfig {
    pub command: Vec<String>,
    pub init_commands: Vec<Vec<String>>,
    pub options: Options,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ExecConfig {
    pub env_file: Option<String>,
    pub env_excludes: Option<Vec<String>>,
    pub options: Options,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Alias {
    pub command: Vec<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub exec: ExecConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub main: MainConfig,
    pub run: RunConfig,
    pub exec: ExecConfig,
    pub aliases: BTreeMap<String, Alias>,
    #[serde(skip)]
    pub directory: PathBuf,
}

impl Config {
    pub fn load(path: Option<&Path>, host: &HostContext) -> Result<Self> {
        let path = match path {
            Some(path) => absolute(path, &host.cwd),
            None => find_file(&host.cwd)
                .ok_or_else(|| Error::Config("Zenvfile not found. Run `zenv init`".into()))?,
        };
        let directory = path
            .parent()
            .ok_or_else(|| Error::Config("Zenvfile has no parent directory".into()))?;
        let parameters = BTreeMap::from([
            ("zenvfilepath", path_string(directory)?),
            ("pwd", path_string(&host.cwd)?),
            ("uid", host.uid.to_string()),
            ("gid", host.gid.to_string()),
            ("tty", host.tty.to_string()),
        ]);
        let mut defaults = parse_config(DEFAULTS)?;
        defaults["exec"]["options"]
            .as_table_mut()
            .expect("builtin exec options are a table")
            .insert("tty".into(), toml::Value::Boolean(host.tty));
        let mut default_parameters = parameters.clone();
        default_parameters.extend([
            ("image", DEFAULT_IMAGE.into()),
            ("id", "zenv".into()),
            ("container_name", directory_name(directory)),
        ]);
        interpolate_value(&mut defaults, &default_parameters);
        let source = fs::read_to_string(&path).map_err(|error| Error::io(&path, error))?;
        let mut custom = parse_config(&source)?;
        interpolate_value(&mut custom, &parameters);
        merge(&mut defaults, custom);
        let mut config: Self = defaults
            .try_into()
            .map_err(|error| Error::Config(format!("{}: {error}", path.display())))?;
        config.directory = directory.to_owned();
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.main.version != 1 {
            return Err(Error::Config(format!(
                "unsupported Zenvfile version {}",
                self.main.version
            )));
        }
        if self.main.image.is_empty() || self.main.name.is_empty() {
            return Err(Error::Config(
                "main.image and main.name must not be empty".into(),
            ));
        }
        validate_command(&self.run.command, "run.command")?;
        for command in &self.run.init_commands {
            validate_command(command, "run.init_commands")?;
        }
        for (name, alias) in &self.aliases {
            validate_command(&alias.command, &format!("alias {name:?}"))?;
        }
        Ok(())
    }
}

/// Tables merge recursively; all other values, including arrays, are replaced.
fn merge(base: &mut toml::Value, custom: toml::Value) {
    match (base, custom) {
        (toml::Value::Table(base), toml::Value::Table(custom)) => {
            for (key, value) in custom {
                match base.get_mut(&key) {
                    Some(base) => merge(base, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, custom) => *base = custom,
    }
}

pub(crate) fn absolute(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path)
    }
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        Error::Config(format!(
            "path cannot be represented in TOML: {}",
            path.display()
        ))
    })
}

fn directory_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "root".into())
}

fn find_file(directory: &Path) -> Option<PathBuf> {
    directory
        .ancestors()
        .map(|directory| directory.join(DEFAULT_FILENAME))
        .find(|path| path.is_file())
}

fn validate_command(command: &[String], field: &str) -> Result<()> {
    if command.first().is_none_or(|first| first.is_empty()) {
        return Err(Error::Config(format!(
            "{field} must contain a nonempty command"
        )));
    }
    Ok(())
}

fn parse_config(source: &str) -> Result<toml::Value> {
    source
        .parse()
        .map_err(|error| Error::Config(format!("invalid Zenvfile: {error}")))
}

fn interpolate_value(value: &mut toml::Value, parameters: &BTreeMap<&str, String>) {
    match value {
        toml::Value::String(text) => *text = interpolate(text, parameters),
        toml::Value::Array(values) => values
            .iter_mut()
            .for_each(|value| interpolate_value(value, parameters)),
        toml::Value::Table(values) => values
            .iter_mut()
            .for_each(|(_, value)| interpolate_value(value, parameters)),
        _ => {}
    }
}

fn interpolate(source: &str, parameters: &BTreeMap<&str, String>) -> String {
    let mut result = String::new();
    let mut rest = source;
    while let Some(index) = rest.find(['{', '}']) {
        result.push_str(&rest[..index]);
        rest = &rest[index..];
        if rest.starts_with("{{") || rest.starts_with("}}") {
            result.push(rest.as_bytes()[0] as char);
            rest = &rest[2..];
        } else if rest.starts_with('{') {
            if let Some(end) = rest.find('}') {
                let token = &rest[1..end];
                result.push_str(parameters.get(token).map_or(&rest[..=end], String::as_str));
                rest = &rest[end + 1..];
            } else {
                result.push_str(rest);
                return result;
            }
        } else {
            result.push('}');
            rest = &rest[1..];
        }
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests;
