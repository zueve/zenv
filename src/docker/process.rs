use std::{
    ffi::OsStr,
    io::{self, Write},
    os::unix::process::ExitStatusExt,
    process::{Command, ExitStatus, Output},
};

use crate::{Error, Result};

use super::DockerCli;

impl DockerCli {
    pub(super) fn command(&self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Command {
        let mut command = Command::new(&self.program);
        command.args(args);
        command
    }

    pub(super) fn trace(&self, command: &Command) {
        if self.debug {
            // Environment values and user arguments may contain secrets.
            eprintln!(
                "Docker: {:?} {:?}",
                command.get_program(),
                command.get_args().next()
            );
        }
    }

    pub(super) fn output(&self, operation: &str, command: &mut Command) -> Result<Output> {
        self.trace(command);
        let output = command
            .output()
            .map_err(|error| Error::runtime(operation, error.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::runtime(
                operation,
                format!("exit code {}: {}", exit_code(output.status), stderr.trim()),
            ));
        }
        Ok(output)
    }

    pub(super) fn action(&self, operation: &str, args: &[&str]) -> Result<()> {
        let output = self.output(operation, &mut self.command(args))?;
        io::stdout()
            .write_all(&output.stdout)
            .map_err(|error| Error::runtime(operation, error.to_string()))?;
        io::stderr()
            .write_all(&output.stderr)
            .map_err(|error| Error::runtime(operation, error.to_string()))?;
        Ok(())
    }
}

pub(super) fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
}
