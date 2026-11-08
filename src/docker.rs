//! Docker CLI adapter. Docker syntax and response formats stay in this module.

use std::ffi::OsString;

mod containers;
mod networks;
mod options;
mod process;

#[derive(Clone, Debug)]
pub struct DockerCli {
    program: OsString,
    debug: bool,
}

impl DockerCli {
    pub fn new(debug: bool) -> Self {
        Self::with_program("docker", debug)
    }

    /// Select an executable, for example a Docker CLI installed outside PATH.
    pub fn with_program(program: impl Into<OsString>, debug: bool) -> Self {
        Self {
            program: program.into(),
            debug,
        }
    }
}

#[cfg(test)]
mod tests;
