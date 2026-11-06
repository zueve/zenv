//! Resolve aliases and prepare runtime requests from project configuration.

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    path::Path,
};

use crate::{
    Error, Result,
    config::{Alias, Config, ExecConfig, HostContext, absolute},
    runtime::{ExecRequest, OptionValue, RunRequest},
};

impl ExecConfig {
    fn overlay(&self, overrides: &Self) -> Self {
        let mut options = self.options.clone();
        options.extend(overrides.options.clone());
        Self {
            env_file: overrides.env_file.clone().or_else(|| self.env_file.clone()),
            env_excludes: overrides
                .env_excludes
                .clone()
                .or_else(|| self.env_excludes.clone()),
            options,
        }
    }
}

impl Config {
    pub fn prepare_run(&self) -> Result<RunRequest> {
        let command: Vec<OsString> = self.run.command.iter().map(OsString::from).collect();
        let (command, _) = self.resolve_alias(&command)?;
        Ok(RunRequest {
            name: self.main.name.clone(),
            image: self.main.image.clone(),
            command,
            options: self.run.options.clone(),
            directory: self.directory.clone(),
        })
    }

    pub fn prepare_exec(
        &self,
        command: &[OsString],
        host: &HostContext,
        defaults: bool,
    ) -> Result<ExecRequest> {
        let (command, alias) = self.resolve_alias(command)?;
        let base = if defaults {
            self.exec.clone()
        } else {
            ExecConfig::default()
        };
        let settings = alias.map_or_else(|| base.clone(), |alias| base.overlay(&alias.exec));
        let mut env = BTreeMap::<OsString, OsString>::new();
        if let Some(file) = settings.env_file.as_deref().filter(|file| !file.is_empty()) {
            let path = absolute(Path::new(file), &host.cwd);
            let reader = fs::File::open(&path).map_err(|error| Error::io(&path, error))?;
            for entry in dotenvy::from_read_iter(reader) {
                let (key, value) =
                    entry.map_err(|error| Error::Config(format!("{}: {error}", path.display())))?;
                let mut row = OsString::from(&key);
                row.push("=");
                row.push(value);
                env.insert(key.into(), row);
            }
        }
        let mut options = settings.options;
        if let Some(configured) = options.remove("env") {
            let OptionValue::List(rows) = configured else {
                return Err(Error::Config(
                    "exec.options.env must be an array of strings".into(),
                ));
            };
            for row in rows {
                let key = row.split_once('=').map_or(row.as_str(), |(key, _)| key);
                if key.is_empty() {
                    return Err(Error::Config(
                        "environment variable names must not be empty".into(),
                    ));
                }
                env.insert(key.into(), row.into());
            }
        }
        for (key, value) in &host.environment {
            if settings
                .env_excludes
                .as_ref()
                .is_some_and(|excludes| excludes.iter().any(|excluded| key == OsStr::new(excluded)))
            {
                continue;
            }
            let mut row = key.clone();
            row.push("=");
            row.push(value);
            env.insert(key.clone(), row);
        }
        Ok(ExecRequest {
            command,
            options,
            env: env.into_values().collect(),
        })
    }

    fn resolve_alias<'a>(
        &'a self,
        command: &[OsString],
    ) -> Result<(Vec<OsString>, Option<&'a Alias>)> {
        let first = command
            .first()
            .filter(|first| !first.is_empty())
            .ok_or_else(|| Error::Config("command must not be empty".into()))?;
        let alias = first.to_str().and_then(|name| self.aliases.get(name));
        let command = match alias {
            Some(alias) => alias
                .command
                .iter()
                .map(OsString::from)
                .chain(command[1..].iter().cloned())
                .collect(),
            None => command.to_vec(),
        };
        Ok((command, alias))
    }
}
