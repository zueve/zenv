import os
import logging as logger
import subprocess
from . import utils
from . import const


def run(config):
    ports_str = ' -p '.join(config['run']['ports'])
    ports_str = f'-p {ports_str}' if ports_str else ''

    volumes = config['run']['volumes']
    volumes_str = ' -v '.join(volumes)
    volumes_str = f'-v {volumes_str}' if volumes_str else ''

    environment = utils.composit_environment(
        config['environment'], config['run']['blacklist_environment'])

    environment_str = ' '.join(
        [f'-e {k}="{v}"' for k, v in environment.items()]
    )

    rm_str = '--rm' if config['run']['autoremove'] else ''

    cmd = (
        f"docker run -d "
        f"--name {config['docker']['container_name']} "
        f"--network {config['run']['network']} "
        f"{ports_str} {volumes_str} {environment_str} {rm_str}"
        f"{config['docker']['image']} {config['run']['command']}"
    )
    with utils.in_directory(os.path.dirname(config['zenvfile_path'])):
        logger.debug(cmd)
        subprocess.run(cmd, shell=True)

    for command in config['run']['init_commands']:
        cmd = 'docker exec {container} {command}'.format(
            container=config['docker']['container_name'],
            command=command
        )
        logger.debug(cmd)
        result = subprocess.run(cmd, shell=True).returncode
        status = 'Success' if result == 0 else 'Fail'
        print(f'{command} -> {status}')

    for command in config['run']['init_user_commands']:
        result = call(config, command)
        status = 'Success' if result == 0 else 'Fail'
        print(f'{command} -> {status}')


def call(config, command, tty=True):
    current_status = status(config['docker']['container_name'])
    if current_status == const.STATUS_NOT_EXIST:
        run(config)
    elif current_status == const.STATUS_STOPED:
        cmd = f'docker start {config["docker"]["container_name"]}'
        logger.debug(cmd)
        subprocess.run(cmd, shell=True)

    # Exec command
    environment = utils.composit_environment(
        config['environment'], config['run']['blacklist_environment'])

    environment_str = ' '.join(
        [f'-e {k}="{v}"' for k, v in environment.items()]
    )

    mode = '-it' if tty else '-i'
    cmd = (
        f"docker exec {mode} -w `pwd` -u `id -u`:`id -g` "
        f"{environment_str} {config['docker']['container_name']} {command}"
    )
    logger.debug(cmd)
    return subprocess.run(cmd, shell=True).returncode


def status(container_name):

    cmd = (
        f"docker ps --all --filter 'name={container_name}' "
        "--format='{{.Status}}'"
    )

    logger.debug(cmd)
    result = subprocess.run(cmd, shell=True, check=True, capture_output=True)
    status = (
        result.stdout.decode().split()[0].upper() if result.stdout else None
    )

    if not status:
        return const.STATUS_NOT_EXIST
    elif status == 'EXITED':
        return const.STATUS_STOPED
    elif status == 'UP':
        return const.STATUS_RUNNING


def version():
    cmd = 'docker version'
    subprocess.run(cmd, shell=True)


def stop(container_name):
    cmd = f'docker stop {container_name}'
    subprocess.run(cmd, shell=True)


def rm(container_name):
    current_status = status(container_name)
    if current_status == const.STATUS_RUNNING:
        stop(container_name)
    if current_status == const.STATUS_NOT_EXIST:
        return
    cmd = f'docker rm {container_name}'
    subprocess.run(cmd, shell=True)


def stop_all(exclude_containers=()):
    """
    Stop all containers started with `zenv-`

    """

    cmd = (
        "docker ps  --format='{{.Names}}'"
    )
    result = subprocess.run(cmd, shell=True, check=True, capture_output=True)

    for container_name in result.stdout.decode().split('\n'):
        if (
            container_name.startswith(const.CONTAINER_PREFIX + '-')
            and container_name not in exclude_containers
        ):
            stop(container_name)
