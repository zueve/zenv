import subprocess
import logging as logger
import json
import sys
from copy import deepcopy

import click
from . import const, networks, utils


def get_command_with_options(command, aliases, exec_params):
    """Prepare docker exec arguments with alias settings overriding defaults."""
    resolved = utils.resolve_alias(command, aliases)
    exec_params = utils.merge_config(deepcopy(resolved['exec']), exec_params)
    env_file = exec_params.get('env_file')
    dotenv_env = utils.load_dotenv(env_file) if env_file else []
    exec_options = dict(exec_params.get('options', {}))
    exec_options['env'] = utils.composit_environment(
        dotenv_env=dotenv_env,
        zenvfile_env=exec_options.get('env', {}),
        blacklist=exec_params.get('env_excludes', ())
    )
    return resolved['command'], utils.build_docker_options(exec_options)


def remove_environment(config):
    """Remove the project container and clean up its configured networks."""
    rm(
        config['main']['name'],
        config['run']['options'].get('network'),
        config['run'].get('manage_network', True),
    )


def _container_for_command(config):
    """Offer to remove an outdated container. Return None when absent."""
    container_name = config['main']['name']
    check_changes = config['run'].get('check_zenvfile_changes', True)
    while True:
        container = inspect_container(container_name)
        if container is None or not check_changes:
            return container
        labels = container['Config'].get('Labels') or {}
        if labels.get(const.CONFIG_HASH_LABEL) == config['main']['config_hash']:
            return container

        click.echo(
            'Warning: container creation settings do not match Zenvfile.',
            err=True,
        )
        if not sys.stdin.isatty():
            raise click.ClickException(
                'Cannot confirm recreation without interactive stdin. '
                'Set run.check_zenvfile_changes = false for noninteractive use.'
            )
        if not click.confirm(
            'Recreate the container with the updated settings?',
            default=False, err=True,
        ):
            return container

        # Another invocation may create a replacement before our next inspect.
        remove_environment(config)


def _create_container(config):
    """Launch the configured container and run its initialization commands."""
    main = config['main']
    setup = config['run']
    aliases = config['aliases']
    options = {'name': main['name'], **setup['options']}
    labels = options.get('label', [])
    if isinstance(labels, str):
        labels = [labels]
    options['label'] = list(labels) + [
        '{}={}'.format(const.CONFIG_HASH_LABEL, main['config_hash'])
    ]
    command = utils.resolve_alias(setup['command'], aliases)['command']
    if setup.get('manage_network', True):
        networks.create(options.get('network'))
    exit_code = run(
        image=main['image'], command=command,
        options=utils.build_docker_options(options), path=main['zenvfilepath'],
    )
    if exit_code:
        return exit_code

    for init_command in setup['init_commands']:
        command, exec_options = get_command_with_options(
            init_command, aliases, {}
        )
        exec_(main['name'], command, exec_options)
    return exit_code


def call(config, command):
    container_name = config['main']['name']
    container = _container_for_command(config)
    command, exec_options = get_command_with_options(
        command, config['aliases'], config['exec']
    )

    if container is None:
        exit_code = _create_container(config)
        if exit_code:
            return exit_code
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
    if container is not None:
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
        networks.remove(network)


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
