Zenv
====

Zenv is container based virtual environments.
The main goal of Zenv is to simplify access to applications inside container, making it seamless in use like native (host-machine) applications

## Usage
```shell
> zenv init
Zenv created!

> # run `ls` command inside contaner
> ze ls
```

## Motivation

As a developer, when set up a project locally, I usually need to install additional applications on the system. Of course, modern package managers such as poetry, pipenv, npm, cargo, ext, perfectly solve for us most of the problems with dependencies within a single stack of technologies. But they do not allow to solve the problem with the installation of system libraries. If you have a lot of projects that require different versions of system libraries, then you have problems

In production, this problem has long been solved by container insulation, and as a rule, it is a `Docker`. Therefore, many of my colleagues use docker-images and docker-compose, not only to run  services in a production environment but also to develop and debug programs on a local machine

Unfortunately, there are several problems along the way:

- Some of your familiar utilities  may not be preinstalled in the container
- If you try to install packages in the process, you will encounter the lack of necessary rights
- Forget about debugging with `print`
- The main thing is lost the usual experience, you can not use your favorite customized shell

Of course, the problems described above are solved by creating a docker image specifically for developing a specific project, `zenv` just helps to create such containers very simply

## Features

- Simplify: all interaction with the container occurs with one short command: `ze`
- Zenv automatically forwarded current directory to the container with the same PWD
- Zenv automatically forwarded current UserID, GroupID to container

  Therefore, files created in the container have the same rights as those created in the native console and you can also use `sudo` to get privileges
  ```shell
  > sudo ze <command>
  ```
  or

  ```shell
  > sudo !!
  ```

- Zenv can forwarded environment vars as is native
    ```shell
    > MYVAR=LIVE!!!! ze env
    ```
- And of course your could combinate native and containerized commands in Unix pipes
  ```shell
  > ze ls | head -7 | ze tail -5
  ```
- Minimal performance impact
- Customization: you can additionally control container parameters using Zenvfile


## Install
  1. Make sure you have the latest version of `Docker` installed
  2. For linux, make sure you have your user’s [rights are allowed](https://docs.docker.com/install/linux/linux-postinstall/) to interact with doker
  3. Make sure that you have python version 3.6 or higher
  4. Execute:
      ```shell
      > sudo pip install zenv-cli
      # or
      > sudo pip3 install zenv-cli
      ```

## How It works
By nature, zenv is a small automated layer over the official docker CLI

### init

Command `zenv init` create `Zenvfile` in current directory. This file describes the relationship with the docker image and container that will be created to execute isolated commands.

By default, used image is `ubuntu: latest`.
But you can change it by setting `-i` flag. For example:
```shell
> zenv init -i python:latest
```
Or edit `Zenvfile` manually


### Execute

Command `zenv exec <COMMAND>` or it shot alias `ze <COMMAND>` run `<COMMAND>` in a container environment. When running the command:
- Zenv checks current container status (running, stopped, removed), and up container if need.
- The container run with the command `sleep infinity`. Therefore it will be easily accessible
- UID and GID that ran the command are pushed into the container. Therefor your could use `sudo ze <COMMAND>`
- When executing the command, the directory from the path where Zenvfile is located is forwarded to the container as is a current PWD. Therefor your could run `ze` from deep into directories relative to Zenvfile
- Environment variables are also thrown into the container as a native.
  ```
  > MYVAR=SUPER ze env
  ```
 To avoid conflicts between host and container variables, all installed system variables are placed    in the blacklist when Zenvfile is created, from which editing can be removed

### Configuration changes

Zenv records a fingerprint of the container creation settings in the Docker label
`zenv.config-hash`. It covers the image, run options, startup command and init
commands, including the aliases and execution settings used by initialization.

If these settings change, Zenv prints a warning to stderr and asks whether to
continue using the existing container (default: no). The same check applies to
containers created before fingerprint tracking. Confirmation happens before
starting the container or executing the command. Accepting preserves the
command's stdout and exit code. With piped or noninteractive stdin, a mismatch
stops execution without reading input.

Fingerprint checking is enabled by default and can be disabled:

```toml
[run]
check_zenvfile_changes = false
```

This disables comparison, warnings and confirmation, including for scripts and
pipes. New containers still receive a fingerprint so checking can be re-enabled
later. Changing this flag does not require recreating the container.

To apply updated creation settings, run `zenv rm` and retry. The next execution
creates a container with the updated configuration. Files in the mounted project
directory remain; changes made only inside the removed container are discarded.

Changing ordinary exec settings, host environment variables, UID/GID, TTY or the
current subdirectory does not require recreating the container. Comments, TOML
key order, alias descriptions and unused aliases do not affect the fingerprint.
The fingerprint tracks configuration declarations, not changes inside image tags
or files referenced by init commands.

### Networks

Set a named network in the existing run options:

```toml
[run]
manage_network = true

[run.options]
network = "project-dev"
```

`run.manage_network` defaults to `true`. Set it to `false` to disable automatic
network creation and deletion. The configured network is still passed to Docker
and must already exist. This switch takes effect immediately and does not
require recreating the container.

Before creating the container, Zenv creates a missing network with the default
bridge driver. Existing networks are reused. Other containers can join the same
network through their Docker or Compose configuration. A list of network names
is also supported.

After `zenv rm` successfully removes the container, Zenv removes unused networks
listed explicitly in the current `run.options.network` that were created by
Zenv. Networks created outside Zenv are preserved.
Networks still referenced by other containers, including stopped containers,
are preserved too. If `run.options.network` is absent, no networks are created
or removed. Other networks attached to the container are left untouched.

Built-in modes (`bridge`, `host`, `none`, `default`) and `container:<name-or-id>`
are passed through without automatic network creation or removal. Extended
Docker network syntax is passed through and requires an existing network.

### Other commands

```shell
> zenv --help
Usage: zenv [OPTIONS] COMMAND [ARGS]...

  ZENV(Zen-env): Containers manager for developer environment

  Usage:

  > zenv init -i centos:latest & ze cat /etc/redhat-release

Options:
  --help  Show this message and exit.

Commands:
  exec      Call some command inside the container
  info      Show current container info
  init      Initialize Environment: create Zenvfile
  rm        Remove container (will stop the container, if need)
  stop      Stop container
  stop-all  Stop all zenv containers
```

## Example install jupyter notebook
```shell
> mkgir notebooks
> cd notebooks
> zenv init -i python:3.7.3
```
After edit Zenvfile to explode notebook ports. Update `ports = []` to `ports = ["8888:8888"]`

```
> sudo ze pip install jupyter numpy scipy matplotlib
```
Run notebook
```
> ze jupyter notebook --ip 0.0.0.0
```
launch your browser with url: http://localhost:8888

Also you could add this commands to Zenvfile:
```toml
[run]
init_commands = [
  [ "__create_user__"],
  ["pip", "install", "jupyter", "numpy", "scipy", "matplotlib"]
]

[commands]
notebook = ["jupyter", "notebook", "--ip", "0.0.0.0"]
```

And launch notebook with command

```
> ze notebook
```

## Zenvfile
```toml
[main]
image = "ubuntu:latest"
name = "zenv-project"    # container name
debug = false            # for show docker commands

[run]
command = [ "__sleep__",]
init_commands = [ [ "__create_user__",],]

[exec]
env_file = ""
env_excludes = [ "TMPDIR",]

[run.options]
volume = [ "{zenvfilepath}:{zenvfilepath}:rw",]
detach = "true"
publish = []

[commands]
__sleep__ = [ "sleep", "365d",]
__create_user__ = [ "useradd", "-m", "-r", "-u", "{uid}", "-g", "{gid}", "zenv",]
```

