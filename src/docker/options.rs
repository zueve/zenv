use std::process::Command;

use crate::{
    Error, Result,
    runtime::{OptionValue, Options},
};

pub(super) fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.starts_with('-') {
        return Err(Error::Config(
            "container name must be nonempty and must not start with '-'".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_options(options: &Options) -> Result<()> {
    for name in options.keys() {
        if name.is_empty()
            || name.starts_with('-')
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(Error::Config(format!(
                "invalid runtime option name: {name:?}"
            )));
        }
    }
    Ok(())
}

pub(super) fn append_options(command: &mut Command, options: &Options) -> Result<()> {
    validate_options(options)?;
    for (name, value) in options {
        let option = format!("--{name}");
        match value {
            OptionValue::Flag(true) => {
                command.arg(option);
            }
            OptionValue::Flag(false) => {}
            OptionValue::Text(value) => {
                command.arg(option).arg(value);
            }
            OptionValue::Integer(value) => {
                command.arg(option).arg(value.to_string());
            }
            OptionValue::List(values) => {
                for value in values {
                    command.arg(&option).arg(value);
                }
            }
        }
    }
    Ok(())
}
