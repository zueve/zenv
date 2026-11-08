use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    Error, Result,
    runtime::{ExecRequest, Options, RunRequest},
};

mod context;
mod environment;
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

impl Alias {
    /// The configured description, or the command when none was provided.
    pub fn description(&self) -> String {
        self.description
            .clone()
            .unwrap_or_else(|| self.command.join(" "))
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub main: MainConfig,
    pub run: RunConfig,
    pub exec: ExecOptions,
    pub aliases: BTreeMap<String, Alias>,
    pub directory: PathBuf,
}

impl Config {
    /// Hash resolved container creation settings without reading external state.
    pub fn run_hash(&self) -> String {
        let init_commands: Vec<_> = self
            .run
            .init_commands
            .iter()
            .map(|request| {
                json!({
                    "command": request.command.iter()
                        .map(|value| value.as_encoded_bytes()).collect::<Vec<_>>(),
                    "options": request.options,
                    "env": request.env.iter()
                        .map(|value| value.as_encoded_bytes()).collect::<Vec<_>>(),
                })
            })
            .collect();
        let settings = json!({
            "project_directory": self.directory.as_os_str().as_encoded_bytes(),
            "image": self.main.image,
            "command": self.run.command,
            "options": self.run.options,
            "init_commands": init_commands,
        });
        format!("{:x}", Sha256::digest(settings.to_string().as_bytes()))
    }

    /// Public aliases in name order; names beginning with an underscore are internal.
    pub fn public_aliases(&self) -> impl Iterator<Item = (&str, &Alias)> {
        self.aliases
            .iter()
            .filter(|(name, _)| !name.starts_with('_'))
            .map(|(name, alias)| (name.as_str(), alias))
    }

    /// Build a runtime request using only the resolved configuration.
    pub fn run_request(&self) -> RunRequest {
        RunRequest {
            fingerprint: Some(self.run_hash()),
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
            Some(names) => names.iter().cloned().collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;
