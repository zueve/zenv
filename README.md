# Zenv

Zenv runs project commands in a persistent container while preserving the current
working directory, user and group IDs, environment variables, and standard streams.
It supports Linux and macOS with Docker installed and running.

## Install

Build with Rust 1.85 or newer:

```sh
cargo install --path . --locked
install -m 755 bin/ze "${CARGO_HOME:-$HOME/.cargo}/bin/ze"
```

Put that directory on your `PATH`. `zenv` is the Rust executable; `ze` is a POSIX sh
script that replaces itself with `zenv exec "$@"`.

## Usage

```sh
zenv init -i python:3.12
ze python --version
MYVAR=hello ze env
ze ls | head
```

`zenv init` writes a `Zenvfile` in the current directory and never overwrites an
existing file. Options are `--image/-i`, `--template/-t` and `--container-name/-c`.
The default image is `ubuntu:latest`; the default name is `zenv-<directory>`.
A supplied name replaces the directory portion of the default template name.

The first execution creates the container and runs its init commands in order.
Subsequent executions reuse it; a stopped container is started automatically.
A failed initialization removes only the container created by that attempt.
Changes to bind-mounted files remain. Creation, startup and cleanup errors are
reported rather than ignored.

```sh
zenv exec --zenvfile /path/to/Zenvfile python --help
zenv info
zenv stop
zenv rm
zenv stop-all --exclude-current
```

`exec`, `info`, `stop`, and `rm` accept `--zenvfile`. Without it, Zenv searches the
current directory and its parents. `stop-all` stops running containers whose names
start with `zenv-`; `--exclude-current/-e` finds the current configuration and skips
its container. It attempts all matching stops and reports any failures.

Arguments after the executed command are passed unchanged, including its flags.
Its exit code is returned to the caller. stdin/stdout/stderr are inherited; TTY
allocation follows stdin. Commands normally run as the invoking UID:GID; running
Zenv as root runs commands as root inside the container.

## Zenvfile v1

Sections and options omitted from a file receive defaults. For example:

```toml
[main]
version = 1
image = "python:3.12"
name = "zenv-notebooks"
debug = false

[run]
command = ["__sleep__"]
init_commands = [
    ["__create_group__"],
    ["__create_user__"],
    ["pip", "install", "jupyter"],
]

[run.options]
volume = ["{zenvfilepath}:{zenvfilepath}:rw"]
detach = true
publish = ["8888:8888"]

[exec]
env_file = ".env"
env_excludes = ["PATH", "HOME", "USER", "SHELL"]

[exec.options]
interactive = true
# tty = false # Optional override; otherwise detected from stdin
workdir = "{pwd}"
user = "{uid}:{gid}"
env = ["APP_MODE=development"]

[aliases.notebook]
command = ["jupyter", "notebook", "--ip", "0.0.0.0"]
description = "Start Jupyter"

[aliases.install]
command = ["pip", "install"]

[aliases.install.exec.options]
user = "0:0"
```

Run aliases with `ze notebook` or `ze install numpy`. Expansion occurs once and
additional arguments are appended. Alias `exec` settings override project settings;
option tables merge, while arrays replace their defaults.

The builtin aliases are `__sleep__`, `__create_group__`, and `__create_user__`.
Init commands run as the container's default user (normally root), unless their
alias overrides execution options. Builtin setup reuses existing UID/GID entries,
including root, and creates missing accounts with `getent`, `groupadd` and
`useradd`. If the name `zenv` is occupied, it uses `zenv-<id>`. Explicit alias
commands are executed as written. Other images can override `init_commands`.

Supported runtime placeholders are `{zenvfilepath}` (project directory), `{pwd}`
(invocation directory), `{uid}`, `{gid}`, and `{tty}`. Unknown placeholders are
preserved; doubled braces escape a placeholder for one interpolation pass.
Custom init templates must be valid TOML and additionally support `{image}`,
`{id}` (`zenv`), and `{container_name}`. An empty `exec.env_excludes = []` is filled
with the host variable names during initialization; a nonempty array is preserved.
Templates are parsed and serialized as TOML, so formatting and comments may change.

Environment precedence, from highest to lowest, is host variables not listed in
`env_excludes`, `exec.options.env`, then `env_file`. Exclusions affect only host
variables. `init` excludes the host variable names present when it runs.
Dotenv paths are relative to the invocation directory; quoted multiline values
are supported. Reading dotenv does not mutate the host process environment.
All execution settings, including environments for aliases and initialization,
are resolved when the configuration is loaded. Each distinct dotenv file is
read and parsed once per load, including when aliases share it or refer to it
through a symlink. Its values are merged with configured and host variables into
the final `ExecOptions` stored in the configuration. Missing or invalid dotenv
files and environment settings fail loading, including those of unused aliases.
The loaded configuration is a snapshot: later file or host environment changes
take effect on the next load. Bare names in `exec.options.env` are resolved from
the host snapshot; names absent from it are unset. Backend extension options
such as `run.options.env` or Docker's `env-file` are passed through to the adapter
and retain that backend's behavior.

Docker options accept TOML booleans, strings, integers, or arrays of repeated
string values. Strings are passed literally, including `"true"` and `"false"`.
`exec.options.env` accepts only an array of strings. `run.options.cidfile` is
reserved for ownership tracking. Set the container name in `main.name`;
conflicting `run.options.name` values are rejected. Debug output reports Docker
operations without printing command arguments or environment secrets.

## Development

```sh
cargo test --locked
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --test docker_live -- --ignored --test-threads=1
```

Normal tests use fake runtimes and fake CLI processes and need no Docker daemon.
The explicitly enabled `docker_live` tests need a working Docker CLI and daemon;
they create uniquely named containers and clean up only their own resources.

The library separates configuration, application logic and runtime adapters.
Configuration types live in `src/config.rs`. Its submodules in `src/config/`
handle loading and validation (`loader`), variable substitution (`template`),
TOML parsing and merging (`toml`), host context (`context`), environment file
reading and composition (`environment`) and configuration creation (`init`).
The loader returns a fully resolved `Config`, including initialization requests
and environments for normal commands and aliases. `Config::run_request` and
`Config::exec_request` build runtime requests solely from these loaded values;
application execution needs no host context or file access. Environment
precedence is host over Zenvfile over `.env`; exclusions apply to host variables.

`ConfigLoader` defines both `init` and `load`. The CLI receives this interface;
`FileConfigLoader` implements it for Zenvfile and dotenv files using a captured
`HostContext`. Other loaders return the same concrete `Config` structure.

Implement `zenv::runtime::ContainerRuntime` and inject it into `zenv::app` to use
another backend. Docker command syntax and response parsing live in
`zenv::docker::DockerCli`. Extension options are interpreted by the adapter;
another backend should reject options it does not support.

All project Rust code forbids `unsafe`. Dependencies may use it internally.
