use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    Error, Result,
    runtime::{ExecRequest, OptionValue, Options, RunRequest},
};

mod context;
mod environment;
mod fingerprint;
mod init;
mod loader;
mod template;
mod toml;

pub use context::HostContext;
pub use loader::{DEFAULT_FILENAME, DEFAULT_IMAGE, FileConfigLoader};

/// Create configuration files and load fully resolved application settings.
pub trait ConfigLoader {
    fn init(
        &self,
        image: &str,
        template: Option<&Path>,
        container_name: Option<&str>,
    ) -> Result<PathBuf>;

    fn load(&self, path: Option<&Path>) -> Result<Config>;
}

#[derive(Clone, Debug, Deserialize)]
pub struct MainConfig {
    pub image: String,
    pub name: String,
    pub debug: bool,
    pub version: u32,
}

#[derive(Clone, Debug)]
pub struct RunConfig {
    pub manage_network: bool,
    pub check_zenvfile_changes: bool,
    pub command: Vec<String>,
    pub init_commands: Vec<ExecRequest>,
    pub options: Options,
}

/// Execution options with environment files and inheritance already resolved.
#[derive(Clone, Debug, Default)]
pub struct ExecOptions {
    pub options: Options,
    pub env: Vec<OsString>,
}

#[derive(Clone, Debug)]
pub struct Alias {
    pub command: Vec<String>,
    pub description: Option<String>,
    pub exec: ExecOptions,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub fingerprint: String,
    pub main: MainConfig,
    pub run: RunConfig,
    pub exec: ExecOptions,
    pub aliases: BTreeMap<String, Alias>,
    pub directory: PathBuf,
}

impl Config {
    /// Build a runtime request using only the resolved configuration.
    pub fn run_request(&self) -> RunRequest {
        RunRequest {
            fingerprint: Some(self.fingerprint.clone()),
            name: self.main.name.clone(),
            image: self.main.image.clone(),
            command: self.run.command.iter().map(OsString::from).collect(),
            options: self.run.options.clone(),
            directory: self.directory.clone(),
        }
    }

    /// Apply an alias and append invocation arguments without reading external state.
    pub fn exec_request(&self, command: &[OsString]) -> Result<ExecRequest> {
        let first = command
            .first()
            .filter(|first| !first.is_empty())
            .ok_or_else(|| Error::Config("command must not be empty".into()))?;
        let alias = first.to_str().and_then(|name| self.aliases.get(name));
        let (command, settings) = match alias {
            Some(alias) => (
                alias
                    .command
                    .iter()
                    .map(OsString::from)
                    .chain(command[1..].iter().cloned())
                    .collect(),
                &alias.exec,
            ),
            None => (command.to_vec(), &self.exec),
        };
        Ok(ExecRequest {
            command,
            options: settings.options.clone(),
            env: settings.env.clone(),
        })
    }

    /// Networks to create or remove under the current management policy.
    pub fn manage_network(&self) -> Vec<String> {
        if !self.run.manage_network {
            return Vec::new();
        }
        match self.run.options.get("network") {
            Some(OptionValue::Text(name)) => vec![name.clone()],
            Some(OptionValue::List(names)) => names.clone(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;
