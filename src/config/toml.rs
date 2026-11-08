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
    pub manage_network: bool,
    pub check_zenvfile_changes: bool,
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
    let mut document = source
        .parse()
        .map_err(|error| Error::Config(format!("invalid Zenvfile: {error}")))?;
    visit_options(&mut document, |options| {
        for (name, value) in options {
            let values = match std::mem::replace(value, Value::Array(Vec::new())) {
                Value::Array(values) => values,
                value => vec![value],
            };
            let mut normalized = Vec::with_capacity(values.len());
            for value in values {
                let text = match value {
                    Value::String(value) => value,
                    Value::Boolean(value) => value.to_string(),
                    Value::Integer(value) => value.to_string(),
                    Value::Float(value) => value.to_string(),
                    Value::Datetime(value) => value.to_string(),
                    Value::Array(_) | Value::Table(_) => {
                        return Err(Error::Config(format!(
                            "runtime option {name:?} values must be scalars"
                        )));
                    }
                };
                normalized.push(Value::String(text));
            }
            *value = Value::Array(normalized);
        }
        Ok(())
    })?;
    Ok(document)
}

fn visit_options(
    document: &mut Value,
    mut visit: impl FnMut(&mut ::toml::map::Map<String, Value>) -> Result<()>,
) -> Result<()> {
    for section in ["run", "exec"] {
        if let Some(options) = document
            .get_mut(section)
            .and_then(|section| section.get_mut("options"))
            .and_then(Value::as_table_mut)
        {
            visit(options)?;
        }
    }
    if let Some(aliases) = document.get_mut("aliases").and_then(Value::as_table_mut) {
        for (_, alias) in aliases.iter_mut() {
            if let Some(options) = alias
                .get_mut("exec")
                .and_then(|exec| exec.get_mut("options"))
                .and_then(Value::as_table_mut)
            {
                visit(options)?;
            }
        }
    }
    Ok(())
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
