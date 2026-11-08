#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

use clap::{Args, Parser, Subcommand};
use zenv::{
    Result, app,
    config::{self, Config, ConfigLoader, FileConfigLoader, HostContext},
    docker::DockerCli,
    runtime::ContainerRuntime,
};

#[derive(Parser)]
#[command(
    name = "zenv",
    version,
    about = "Container-based development environments"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Args)]
struct ConfigPath {
    /// Path to Zenvfile (otherwise search the current directory and its parents)
    #[arg(long)]
    zenvfile: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize environment: create Zenvfile
    Init {
        #[arg(short = 'i', long, default_value = config::DEFAULT_IMAGE)]
        image: String,
        #[arg(short = 't', long)]
        template: Option<PathBuf>,
        #[arg(short = 'c', long)]
        container_name: Option<String>,
    },
    /// Call a command inside the container
    #[command(disable_help_flag = true)]
    Exec {
        #[command(flatten)]
        config: ConfigPath,
        #[arg(required = true, num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<OsString>,
    },
    /// Show current container info
    Info(ConfigPath),
    /// Stop container
    Stop(ConfigPath),
    /// Remove container (stopping it if necessary)
    Rm(ConfigPath),
    /// Stop all containers whose names start with zenv-
    StopAll {
        #[command(flatten)]
        config: ConfigPath,
        #[arg(short = 'e', long)]
        exclude_current: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = HostContext::capture().and_then(|host| run(cli, &FileConfigLoader::new(&host)));
    match result {
        Ok(code) => ExitCode::from(code as u8),
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli, loader: &dyn ConfigLoader) -> Result<i32> {
    match cli.command {
        Commands::Init {
            image,
            template,
            container_name,
        } => {
            loader.init(&image, template.as_deref(), container_name.as_deref())?;
            println!("Zenvfile created");
        }
        Commands::Exec { config, command } => {
            let config = loader.load(config.zenvfile.as_deref())?;
            return app::execute(&DockerCli::new(config.main.debug), &config, &command);
        }
        Commands::Info(path) => {
            let config = loader.load(path.zenvfile.as_deref())?;
            let runtime = DockerCli::new(config.main.debug);
            let container = runtime.inspect(&config.main.name)?;
            let mut output = io::stdout().lock();
            let text = info(&config, container.as_ref());
            if let Err(source) = output.write_all(text.as_bytes()) {
                if source.kind() != io::ErrorKind::BrokenPipe {
                    return Err(zenv::Error::io("stdout", source));
                }
            }
        }
        Commands::Stop(path) => {
            let config = loader.load(path.zenvfile.as_deref())?;
            app::stop(&DockerCli::new(config.main.debug), &config.main.name)?;
        }
        Commands::Rm(path) => {
            let config = loader.load(path.zenvfile.as_deref())?;
            app::remove(&DockerCli::new(config.main.debug), &config.main.name)?;
        }
        Commands::StopAll {
            config,
            exclude_current,
        } => {
            let config = if exclude_current {
                Some(loader.load(config.zenvfile.as_deref())?)
            } else {
                None
            };
            let runtime = DockerCli::new(config.as_ref().is_some_and(|c| c.main.debug));
            app::stop_all(&runtime, config.as_ref().map(|c| c.main.name.as_str()))?;
        }
    }
    Ok(0)
}

fn info(config: &Config, container: Option<&zenv::runtime::Container>) -> String {
    use std::fmt::Write;

    let mut text = format!(
        "Zenvfile: {}\nImage: {}\nContainer: {}\nStatus: {}\n",
        config.directory.display(),
        config.main.image,
        config.main.name,
        container.map_or_else(|| "Not_exist".to_owned(), |c| c.state.to_string()),
    );
    let aliases: Vec<_> = config
        .aliases
        .iter()
        .filter(|(name, _)| !name.starts_with('_'))
        .collect();
    if !aliases.is_empty() {
        text.push_str("Aliases:\n");
    }
    for (name, alias) in aliases {
        let description = alias.description.clone().unwrap_or_else(|| {
            let command = alias.command.join(" ");
            if command.chars().count() > 80 {
                format!("{} ...", command.chars().take(74).collect::<String>())
            } else {
                command
            }
        });
        writeln!(&mut text, " - {name}: {description}").expect("writing to a String cannot fail");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_preserves_command_options() {
        let cli = Cli::try_parse_from([
            "zenv",
            "exec",
            "--zenvfile",
            "custom",
            "env",
            "--help",
            "--zenvfile",
            "literal",
            "",
            "a b",
        ])
        .unwrap();
        let Commands::Exec { config, command } = cli.command else {
            panic!("exec expected")
        };
        assert_eq!(config.zenvfile, Some(PathBuf::from("custom")));
        assert_eq!(
            command,
            ["env", "--help", "--zenvfile", "literal", "", "a b"].map(OsString::from)
        );
    }

    #[test]
    fn exec_help_is_a_command() {
        let cli = Cli::try_parse_from(["zenv", "exec", "--help"]).unwrap();
        assert!(
            matches!(cli.command, Commands::Exec { command, .. } if command == [OsString::from("--help")])
        );
    }

    #[test]
    fn requires_a_command() {
        assert!(Cli::try_parse_from(["zenv", "exec"]).is_err());
    }
}
