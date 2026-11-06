use std::{fs::OpenOptions, io::Write};

use super::*;

/// Initialize without overwriting an existing file, including in concurrent invocations.
pub fn init(
    host: &HostContext,
    image: &str,
    template: Option<&Path>,
    container_name: Option<&str>,
) -> Result<PathBuf> {
    let source = match template {
        Some(path) => {
            let path = absolute(path, &host.cwd);
            fs::read_to_string(&path).map_err(|error| Error::io(&path, error))?
        }
        None => DEFAULTS.into(),
    };
    let excluded: Vec<&str> = host
        .environment
        .keys()
        .map(|key| {
            key.to_str().ok_or_else(|| {
                Error::Config("cannot store non-UTF-8 environment variable name in Zenvfile".into())
            })
        })
        .collect::<Result<_>>()?;
    let excludes = toml::Value::Array(
        excluded
            .into_iter()
            .map(|key| toml::Value::String(key.into()))
            .collect(),
    );
    let mut document = parse_config(&source)?;
    if let Some(exec) = document.get_mut("exec").and_then(toml::Value::as_table_mut) {
        if exec
            .get("env_excludes")
            .and_then(toml::Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            exec.insert("env_excludes".into(), excludes);
        }
    }
    let parameters = BTreeMap::from([
        (
            "image",
            if image.is_empty() {
                DEFAULT_IMAGE
            } else {
                image
            }
            .to_owned(),
        ),
        ("id", "zenv".into()),
        (
            "container_name",
            container_name
                .map(str::to_owned)
                .unwrap_or_else(|| directory_name(&host.cwd)),
        ),
    ]);
    interpolate_value(&mut document, &parameters);
    let content =
        toml::to_string_pretty(&document).map_err(|error| Error::Config(error.to_string()))?;
    let path = host.cwd.join(DEFAULT_FILENAME);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| Error::io(&path, error))?;
    file.write_all(content.as_bytes())
        .map_err(|error| Error::io(&path, error))?;
    Ok(path)
}
