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

