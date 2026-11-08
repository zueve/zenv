//! Fingerprint container creation declarations before host-specific substitutions.

use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

use ::toml::Value;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{Error, Result};

use super::{
    loader::{DEFAULT_IMAGE, DEFAULTS},
    template, toml,
};

pub(super) fn config_hash(source: &str, directory: &Path) -> Result<String> {
    let parameters = BTreeMap::from([
        ("uid", "__zenv_runtime_uid__".into()),
        ("gid", "__zenv_runtime_gid__".into()),
        ("pwd", "__zenv_runtime_pwd__".into()),
        ("tty", "__zenv_runtime_tty__".into()),
        ("zenvfilepath", "__zenv_project_directory__".into()),
    ]);
    let mut defaults = toml::parse(DEFAULTS)?;
    let mut default_parameters = parameters.clone();
    default_parameters.extend([("image", DEFAULT_IMAGE.into()), ("id", "zenv".into())]);
    template::interpolate_value(&mut defaults, &default_parameters);
    let mut custom = toml::parse(source)?;
    template::interpolate_value(&mut custom, &parameters);
    toml::merge(&mut defaults, custom);

    let aliases = &defaults["aliases"];
    let expand = |command: &Value| {
        let command = command.as_array().expect("validated command is an array");
        let alias = aliases.get(command[0].as_str().expect("validated command is text"));
        let mut expanded = alias
            .and_then(|alias| alias.get("command"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| command[..1].to_vec());
        expanded.extend_from_slice(&command[1..]);
        expanded
    };
    let init_commands: Vec<_> = defaults["run"]["init_commands"]
        .as_array()
        .expect("validated init commands are an array")
        .iter()
        .map(|command| {
            let name = command[0].as_str().expect("validated command is text");
            let exec = aliases
                .get(name)
                .and_then(|alias| alias.get("exec"))
                .cloned()
                .unwrap_or_else(|| Value::Table(Default::default()));
            json!({"command": expand(command), "exec": exec})
        })
        .collect();
    let setup = json!({
        "project_directory": normalize(directory),
        "image": defaults["main"]["image"],
        "command": expand(&defaults["run"]["command"]),
        "options": defaults["run"]["options"],
        "init_commands": init_commands,
    });
    let content = serde_json::to_vec(&setup)
        .map_err(|error| Error::Config(format!("cannot fingerprint configuration: {error}")))?;
    Ok(format!("{:x}", Sha256::digest(content)))
}

fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}
