use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

use crate::{Error, Result};

use super::{ExecOptions, HostContext, loader::absolute, toml::ExecSettings};

type Environment = BTreeMap<OsString, OsString>;
pub(super) type EnvFiles = BTreeMap<PathBuf, Environment>;

pub(super) fn load_files<'a>(
    settings: impl IntoIterator<Item = &'a ExecSettings>,
    cwd: &Path,
) -> Result<EnvFiles> {
    load_files_with(settings, cwd, read_env_file)
}

fn load_files_with<'a>(
    settings: impl IntoIterator<Item = &'a ExecSettings>,
    cwd: &Path,
    mut read: impl FnMut(&Path) -> Result<Environment>,
) -> Result<EnvFiles> {
    let paths: BTreeSet<_> = settings
        .into_iter()
        .filter_map(|settings| settings.env_file.as_deref())
        .filter(|file| !file.is_empty())
        .map(|file| absolute(Path::new(file), cwd))
        .collect();
    let mut contents = BTreeMap::new();
    let mut files = BTreeMap::new();
    for path in paths {
        let canonical = fs::canonicalize(&path).map_err(|error| Error::io(&path, error))?;
        let env = match contents.entry(canonical) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(read(&path)?),
        };
        files.insert(path, env.clone());
    }
    Ok(files)
}

fn read_env_file(path: &Path) -> Result<Environment> {
    let reader = fs::File::open(path).map_err(|error| Error::io(path, error))?;
    let mut env = BTreeMap::new();
    for entry in dotenvy::from_read_iter(reader) {
        let (key, value) =
            entry.map_err(|error| Error::Config(format!("{}: {error}", path.display())))?;
        let mut row = OsString::from(&key);
        row.push("=");
        row.push(value);
        env.insert(key.into(), row);
    }
    Ok(env)
}

pub(super) fn compose(
    settings: ExecSettings,
    host: &HostContext,
    files: &EnvFiles,
) -> Result<ExecOptions> {
    let mut env = match settings.env_file.as_deref().filter(|file| !file.is_empty()) {
        Some(file) => files
            .get(&absolute(Path::new(file), &host.cwd))
            .expect("dotenv files are loaded before composing environments")
            .clone(),
        None => BTreeMap::new(),
    };
    let mut options = settings.options;
    if let Some(configured) = options.remove("env") {
        for row in configured {
            let key = row.split_once('=').map_or(row.as_str(), |(key, _)| key);
            if key.is_empty() {
                return Err(Error::Config(
                    "environment variable names must not be empty".into(),
                ));
            }
            let mut value = OsString::from(&row);
            if !row.contains('=') {
                if let Some(inherited) = host.environment.get(OsStr::new(key)) {
                    value.push("=");
                    value.push(inherited);
                }
            }
            env.insert(key.into(), value);
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
    Ok(ExecOptions {
        options,
        env: env.into_values().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexSet;
    use std::os::unix::fs::symlink;

    #[test]
    fn shared_files_are_preloaded_and_composition_needs_no_file_access() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(".env");
        fs::write(&path, "KEY=dotenv\nFILE_ONLY=dotenv\n").unwrap();
        let link = directory.path().join("alias.env");
        symlink(&path, &link).unwrap();
        let base = ExecSettings {
            env_file: Some(".env".into()),
            ..ExecSettings::default()
        };
        let alias = ExecSettings {
            env_file: Some("alias.env".into()),
            env_excludes: Some(vec!["KEY".into()]),
            options: BTreeMap::from([("env".into(), IndexSet::from(["KEY=config".into()]))]),
        };
        let mut reads = 0;
        let files = load_files_with([&base, &base, &alias], directory.path(), |path| {
            reads += 1;
            read_env_file(path)
        })
        .unwrap();
        assert_eq!(reads, 1);
        fs::remove_file(&path).unwrap();
        fs::remove_file(&link).unwrap();
        let host = HostContext {
            cwd: directory.path().to_owned(),
            uid: 123,
            gid: 456,
            tty: false,
            environment: BTreeMap::from([("KEY".into(), "host".into())]),
        };
        assert_eq!(
            compose(base, &host, &files).unwrap().env,
            ["FILE_ONLY=dotenv", "KEY=host"]
        );
        assert_eq!(
            compose(alias, &host, &files).unwrap().env,
            ["FILE_ONLY=dotenv", "KEY=config"]
        );

        fs::write(&path, "KEY=updated\n").unwrap();
        let settings = ExecSettings {
            env_file: Some(".env".into()),
            ..ExecSettings::default()
        };
        let reloaded = load_files([&settings], directory.path()).unwrap();
        assert_eq!(reloaded[&path][OsStr::new("KEY")], "KEY=updated");
    }
}
