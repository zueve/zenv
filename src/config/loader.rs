//! Assemble a validated configuration from defaults, a Zenvfile and host context.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use crate::{Error, Result, runtime::ExecRequest};

use super::{
    Alias, Config, ConfigLoader, HostContext, RunConfig, environment, fingerprint, init, template,
    toml::{self, AliasSettings, Document},
};

pub const DEFAULT_FILENAME: &str = "Zenvfile";
pub const DEFAULT_IMAGE: &str = "ubuntu:latest";
pub(super) const DEFAULTS: &str = include_str!("defaults.toml");

/// Filesystem loader using a captured host context for initialization and loading.
pub struct FileConfigLoader<'a> {
    host: &'a HostContext,
}

impl<'a> FileConfigLoader<'a> {
    pub fn new(host: &'a HostContext) -> Self {
        Self { host }
    }
}

impl ConfigLoader for FileConfigLoader<'_> {
    fn init(
        &self,
        image: &str,
        template: Option<&Path>,
        container_name: Option<&str>,
    ) -> Result<PathBuf> {
        init::init(self.host, image, template, container_name)
    }

    fn load(&self, path: Option<&Path>) -> Result<Config> {
        let host = self.host;
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
        let mut defaults = toml::parse(DEFAULTS)?;
        defaults["exec"]["options"]
            .as_table_mut()
            .expect("builtin exec options are a table")
            .insert("tty".into(), ::toml::Value::Boolean(host.tty));
        let mut default_parameters = parameters.clone();
        default_parameters.extend([
            ("image", DEFAULT_IMAGE.into()),
            ("id", "zenv".into()),
            ("container_name", directory_name(directory)),
        ]);
        template::interpolate_value(&mut defaults, &default_parameters);
        let source = fs::read_to_string(&path).map_err(|error| Error::io(&path, error))?;
        let mut custom = toml::parse(&source)?;
        template::interpolate_value(&mut custom, &parameters);
        toml::merge(&mut defaults, custom);
        let document: Document = defaults
            .try_into()
            .map_err(|error| Error::Config(format!("{}: {error}", path.display())))?;
        document.validate()?;
        let fingerprint = fingerprint::config_hash(&source, directory)?;

        let env_files = environment::load_files(
            std::iter::once(&document.exec)
                .chain(document.aliases.values().map(|alias| &alias.exec)),
            &host.cwd,
        )?;
        let command = expand_command(&document.run.command, &document.aliases);
        let init_commands = document
            .run
            .init_commands
            .iter()
            .map(|command| {
                let settings = document
                    .aliases
                    .get(&command[0])
                    .map(|alias| alias.exec.clone())
                    .unwrap_or_default();
                let exec = environment::compose(settings, host, &env_files)?;
                Ok(ExecRequest {
                    command: expand_command(command, &document.aliases)
                        .into_iter()
                        .map(OsString::from)
                        .collect(),
                    options: exec.options,
                    env: exec.env,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let exec = environment::compose(document.exec.clone(), host, &env_files)?;
        let aliases = document
            .aliases
            .iter()
            .map(|(name, alias)| {
                Ok((
                    name.clone(),
                    Alias {
                        command: alias.command.clone(),
                        description: alias.description.clone(),
                        exec: environment::compose(
                            document.exec.overlay(&alias.exec),
                            host,
                            &env_files,
                        )?,
                    },
                ))
            })
            .collect::<Result<_>>()?;

        Ok(Config {
            fingerprint,
            main: document.main,
            run: RunConfig {
                manage_network: document.run.manage_network,
                check_zenvfile_changes: document.run.check_zenvfile_changes,
                command,
                init_commands,
                options: document.run.options,
            },
            exec,
            aliases,
            directory: directory.to_owned(),
        })
    }
}

impl Document {
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

fn expand_command(command: &[String], aliases: &BTreeMap<String, AliasSettings>) -> Vec<String> {
    match aliases.get(&command[0]) {
        Some(alias) => alias.command.iter().chain(&command[1..]).cloned().collect(),
        None => command.to_vec(),
    }
}

pub(super) fn absolute(path: &Path, cwd: &Path) -> PathBuf {
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

pub(super) fn directory_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "root".into())
}

pub(super) fn find_file(directory: &Path) -> Option<PathBuf> {
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
