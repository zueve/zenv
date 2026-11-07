import subprocess
import logging as logger
import json
import sys
from copy import deepcopy

import click
from . import const, networks, utils


def get_command_with_options(command, aliases, exec_params):
    """
    Find command by aliases and build exec docker options

    """
    exec_params = deepcopy(exec_params)
    if command[0] in aliases:
        key = command[0]
        command = aliases[key]['command'] + list(command[1:])
        command_exec_params = deepcopy(aliases[key].get('exec', {}))
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
    container = inspect_container(container_name)
    if (container is not None
            and config['run'].get('check_zenvfile_changes', True)):
        labels = container['Config'].get('Labels') or {}
        saved_hash = labels.get(const.CONFIG_HASH_LABEL)
        if saved_hash != config['main']['config_hash']:
            click.echo(
                'Warning: container creation settings do not match Zenvfile. '
                'Run `zenv rm` to apply the updated settings.',
                err=True,
            )
            if not sys.stdin.isatty():
                raise click.ClickException(
                    'Cannot confirm continuation without interactive stdin. '
                    'Set run.check_zenvfile_changes = false for '
                    'noninteractive use.'
                )
            click.confirm(
                'Continue using the existing container?',
                default=False, abort=True, err=True,
            )

    # composite environments
    command, exec_options = get_command_with_options(
        command, config['aliases'], config['exec']
    )

    if container is None:
        options = {'name': container_name, **config['run']['options']}
        labels = options.get('label', [])
        if isinstance(labels, str):
            labels = [labels]
        options['label'] = list(labels) + [
            '{}={}'.format(
                const.CONFIG_HASH_LABEL, config['main']['config_hash']
            )
        ]
        run_command, _ = get_command_with_options(
            config['run']['command'], config['aliases'], {})
        created_networks = (
            networks.ensure_networks(options.get('network'))
            if config['run'].get('manage_network', True) else []
        )
        exit_code = None
        try:
            exit_code = run(
                image=config['main']['image'],
                command=run_command,
                options=utils.build_docker_options(options),
                path=config['main']['zenvfilepath']
            )
        finally:
            if exit_code != 0:
                for network_id in created_networks:
                    try:
                        networks.remove_if_unused(network_id)
                    except click.ClickException as error:
                        click.echo('Warning: {}'.format(error), err=True)
        if exit_code:
            return exit_code

        # run init commands:
        for init_command in config['run']['init_commands']:
            init_command, init_options = get_command_with_options(
                init_command, config['aliases'], {}
            )
            exec_(container_name, init_command, init_options)

    elif container['State']['Status'] in ('exited', 'created'):
        cmd = ['docker', 'start', container_name]
        logger.debug(cmd)
        result = subprocess.run(cmd)
        if result.returncode:
            return result.returncode

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


def inspect_container(container_name):
    cmd = ['docker', 'container', 'inspect', '--', container_name]
    logger.debug(cmd)
    result = subprocess.run(cmd, capture_output=True)
    if result.returncode == 0:
        return json.loads(result.stdout)[0]

    # Verify absence with a daemon query, without parsing error text.
    names = subprocess.run(
        ['docker', 'ps', '--all', '--format', '{{.Names}}'],
        capture_output=True,
    )
    if names.returncode:
        raise click.ClickException(names.stderr.decode().strip())
    if container_name not in names.stdout.decode().splitlines():
        return None
    raise click.ClickException(result.stderr.decode().strip())


def status(container_name):
    container = inspect_container(container_name)
    if container is None:
        return const.STATUS_NOT_EXIST
    state = container['State']['Status']
    return {
        'running': const.STATUS_RUNNING,
        'exited': const.STATUS_STOPED,
    }.get(state, state)


def version():
    cmd = 'docker version'
    subprocess.run(cmd, shell=True)


def stop(container_name):
    cmd = f'docker stop {container_name}'
    subprocess.run(cmd, shell=True)


def rm(container_name, network=None, manage_network=True):
    container = inspect_container(container_name)
    if container is None:
        return
    container_id = container['Id']
    if container['State']['Status'] == 'running':
        cmd = ['docker', 'stop', '--', container_id]
        logger.debug(cmd)
        if subprocess.run(cmd).returncode:
            raise click.ClickException('Could not stop the container.')
    cmd = ['docker', 'rm', '--', container_id]
    logger.debug(cmd)
    if subprocess.run(cmd).returncode:
        raise click.ClickException('Could not remove the container.')
    if manage_network:
        for network_name in networks.configured_networks(network):
            networks.remove_if_unused(network_name)


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
