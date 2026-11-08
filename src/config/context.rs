use std::{collections::BTreeMap, ffi::OsString, io::IsTerminal, path::PathBuf};

use crate::{Error, Result};

/// A snapshot allows configuration to be resolved without modifying global state.
#[derive(Clone, Debug)]
pub struct HostContext {
    pub cwd: PathBuf,
    pub uid: u32,
    pub gid: u32,
    pub tty: bool,
    pub environment: BTreeMap<OsString, OsString>,
}

impl HostContext {
    pub fn capture() -> Result<Self> {
        Ok(Self {
            cwd: std::env::current_dir().map_err(|error| Error::io(".", error))?,
            uid: rustix::process::getuid().as_raw(),
            gid: rustix::process::getgid().as_raw(),
            tty: std::io::stdin().is_terminal(),
            environment: std::env::vars_os().collect(),
        })
    }
}
