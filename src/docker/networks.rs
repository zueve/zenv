use std::process::Stdio;

use super::DockerCli;

impl DockerCli {
    pub(super) fn network_action(&self, args: &[&str], names: &[String]) {
        for name in names {
            let mut command = self.command(["network"]);
            command
                .args(args)
                .arg("--")
                .arg(name)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            self.trace(&command);
            let _ = command.status();
        }
    }
}
