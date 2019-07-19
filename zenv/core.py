import subprocess
import logging as logger
from . import const, utils


def call(config, command, environments):
    container_name = config['main']['name']
    container_status = status(container_name)

    aliases = utils.Aliases(config['commands'])

    # composite environments
    exec_options = config['exec']['options']
    exec_options['env'] = utils.composit_environment(
        file_env=environments,
        zenvfile_env=environments + exec_options.get('env', []),
        blacklist=config['exec']['env_excludes']
    )
    exec_options = utils.build_docker_options(exec_options)

    if container_status == const.STATUS_NOT_EXIST:
        options = {
            'name': container_name,
            **config['run']['options']
        }
        run(
            image=config['main']['image'],
            command=aliases[config['run']['command']],
            options=utils.build_docker_options(options),
            path=config['main']['zenvfilepath']
        )

        # run init commands:
        for init_command in config['run']['init_commands']:
            exec_(container_name, aliases[init_command], [])

    elif container_status == const.STATUS_STOPED:
        cmd = ['docker', 'start', container_name]
        logger.debug(cmd)
        subprocess.run(cmd)

    exec_(container_name, aliases[command], exec_options)


def run(image, command, options, path):
    cmd = ['docker', 'run', *options, image, *command]

    with utils.in_directory(path):
        logger.debug(cmd)
        subprocess.run(cmd)


def exec_(container_name, command, options):
    cmd = ('docker', 'exec', *options, container_name, *command)
    logger.debug(cmd)
    return subprocess.run(cmd).returncode


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
