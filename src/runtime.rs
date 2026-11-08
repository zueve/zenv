use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

use serde::Deserialize;

use crate::Result;

pub const CONFIG_HASH_LABEL: &str = "zenv.config-hash";
pub const MANAGED_NETWORK_LABEL: &str = "zenv.managed-network";

/// Backend-specific extension options, interpreted only by the runtime adapter.
pub type Options = BTreeMap<String, OptionValue>;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum OptionValue {
    Flag(bool),
    Text(String),
    List(Vec<String>),
    Integer(i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainerState {
    Created,
    Running,
    Exited,
    Paused,
    Restarting,
    Removing,
    Dead,
    Unknown(String),
}

impl std::fmt::Display for ContainerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Created => "Created",
            Self::Running => "Running",
            Self::Exited => "Stoped",
            Self::Paused => "Paused",
            Self::Restarting => "Restarting",
            Self::Removing => "Removing",
            Self::Dead => "Dead",
            Self::Unknown(state) => state,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub state: ContainerState,
    pub fingerprint: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RunRequest {
    pub name: String,
    pub image: String,
    pub command: Vec<OsString>,
    pub options: Options,
    pub directory: PathBuf,
    pub fingerprint: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ExecRequest {
    pub command: Vec<OsString>,
    pub options: Options,
    /// Resolved environment entries in KEY=VALUE form (or KEY to unset it).
    pub env: Vec<OsString>,
}

/// Implement this trait to replace the container backend without changing the app.
pub trait ContainerRuntime {
    /// Check request options locally, without launching processes or changing resources.
    fn validate_run(&self, request: &RunRequest) -> Result<()>;
    fn validate_exec(&self, request: &ExecRequest) -> Result<()>;
    fn inspect(&self, name: &str) -> Result<Option<Container>>;
    /// Return the ID created by this call. On failure, clean up any container
    /// created by the call; never remove a preexisting container with the same name.
    fn run(&self, request: &RunRequest) -> Result<Container>;
    fn start(&self, id: &str) -> Result<()>;
    fn exec(&self, id: &str, request: &ExecRequest) -> Result<i32>;
    fn stop(&self, id: &str) -> Result<()>;
    /// Force removal supports rollback and explicit removal of active containers.
    fn remove(&self, id: &str, force: bool) -> Result<()>;
    fn list_running(&self) -> Result<Vec<Container>>;
    /// Best-effort operations; the backend decides whether a network can be changed.
    fn create_networks(&self, names: &[String]);
    fn remove_networks(&self, names: &[String]);
}
