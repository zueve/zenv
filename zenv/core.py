import subprocess
import logging as logger
from . import const, utils


def get_command_with_options(command, aliases, exec_params):
    """
    Find command by aliases and build exec docker options

    """
    if command[0] in aliases:
        key = command[0]
        command = aliases[key]['command'] + list(command[1:])
        command_exec_params = aliases[key].get('exec', {})
        exec_params = utils.merge_config(command_exec_params, exec_params)
    dotenv_env = (
        utils.load_dotenv(exec_params['env_file'])
        if 'env_file' in exec_params and exec_params['env_file'] else {}
    )
    exec_options = exec_params.get('options', {})
    exec_options['env'] = utils.composit_environment(
        dotenv_env=dotenv_env,
        zenvfile_env=exec_options.get('env', {}),
        blacklist=exec_params.get('env_excludes', {})
    )
    docker_exec_options = utils.build_docker_options(exec_options)
    return command, docker_exec_options


def call(config, command):
    container_name = config['main']['name']
    container_status = status(container_name)

    # composite environments
    command, exec_options = get_command_with_options(
        command, config['aliases'], config['exec']
    )

    if container_status == const.STATUS_NOT_EXIST:
        options = {'name': container_name, **config['run']['options']}
        run_command, _ = get_command_with_options(
            config['run']['command'], config['aliases'], {})
        run(
            image=config['main']['image'],
            command=run_command,
            options=utils.build_docker_options(options),
            path=config['main']['zenvfilepath']
        )

        # run init commands:
        for init_command in config['run']['init_commands']:
            init_command, init_options = get_command_with_options(
                init_command, config['aliases'], {}
            )
            exec_(container_name, init_command, init_options)

    elif container_status == const.STATUS_STOPED:
        cmd = ['docker', 'start', container_name]
        logger.debug(cmd)
        subprocess.run(cmd)

    return exec_(container_name, command, exec_options)


def run(image, command, options, path):
    cmd = ['docker', 'run', *options, image, *command]

    with utils.in_directory(path):
        logger.debug(cmd)
        result = subprocess.run(cmd)
    return result.returncode


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
