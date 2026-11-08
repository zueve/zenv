//! Parse and serialize TOML, and overlay project settings on defaults.

use ::toml::Value;
use serde::Deserialize;

use crate::{Error, Result, runtime::Options};

use super::MainConfig;

#[derive(Deserialize)]
pub(super) struct Document {
    pub main: MainConfig,
    pub run: RunSettings,
    pub exec: ExecSettings,
    pub aliases: std::collections::BTreeMap<String, AliasSettings>,
}

#[derive(Deserialize)]
pub(super) struct RunSettings {
    pub command: Vec<String>,
    pub init_commands: Vec<Vec<String>>,
    pub options: Options,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct ExecSettings {
    pub env_file: Option<String>,
    pub env_excludes: Option<Vec<String>>,
    pub options: Options,
}

impl ExecSettings {
    pub(super) fn overlay(&self, overrides: &Self) -> Self {
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

#[derive(Deserialize)]
pub(super) struct AliasSettings {
    pub command: Vec<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub exec: ExecSettings,
}

pub(super) fn parse(source: &str) -> Result<Value> {
    source
        .parse()
        .map_err(|error| Error::Config(format!("invalid Zenvfile: {error}")))
}

pub(super) fn serialize(document: &Value) -> Result<String> {
    ::toml::to_string_pretty(document).map_err(|error| Error::Config(error.to_string()))
}

/// Tables merge recursively; all other values, including arrays, are replaced.
pub(super) fn merge(base: &mut Value, custom: Value) {
    match (base, custom) {
        (Value::Table(base), Value::Table(custom)) => {
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
