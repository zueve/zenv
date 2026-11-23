import os
import sys
import logging
from pathlib import Path
from contextlib import contextmanager
import click
import toml
from dotenv import parser

from . import const


class Default(dict):
    def __missing__(self, key):
        return '{' + key + '}'


def resolve_alias(command, aliases):
    """Expand the first word once and return the declared exec overrides."""
    alias = aliases.get(command[0])
    if alias is None:
        return {'command': command, 'exec': {}}
    return {
        'command': alias['command'] + list(command[1:]),
        'exec': alias.get('exec', {}),
    }


def load_dotenv(dotenvfile):
    try:
        with open(dotenvfile) as file:
            environments = [
                f'{bind.key}={bind.value}'
                if bind.value is not None else bind.key
                for bind in parser.parse_stream(file)
                if not bind.error and bind.key is not None
            ]
    except FileNotFoundError:
        raise click.ClickException(f'Env file not found {dotenvfile}')
    return environments


def get_config(zenvfile=None):
    from .fingerprint import config_hash

    if not zenvfile:
        zenvfile = find_file(os.getcwd(), fname=const.DEFAULT_FILENAME)
    if not zenvfile:
        raise click.ClickException('Zenvfile don\'t find. Make `zenv init`')

    zenvfile = os.path.abspath(zenvfile)
    source = Path(zenvfile).read_text()

    # Distinct symbols retain active placeholders while unescaping TOML braces.
    symbolic_params = Default(
        uid='__zenv_runtime_uid__', gid='__zenv_runtime_gid__',
        pwd='__zenv_runtime_pwd__', tty='__zenv_runtime_tty__',
        zenvfilepath='__zenv_project_directory__', env_excludes='[]',
    )
    declared_config = merge_config(
        toml.loads(source.format_map(symbolic_params)),
        toml.loads(const.CONFIG_TEMPLATE.format_map(symbolic_params)),
    )
    creation_hash = config_hash(
        declared_config, os.path.dirname(zenvfile)
    )

    params = Default(
        zenvfilepath=os.path.dirname(zenvfile),
        pwd=os.getcwd(),
        uid=os.getuid(),
        gid=os.getgid(),
        tty='true' if sys.stdin.isatty() else 'false',
        env_excludes='[]'
    )

    content = source.format_map(params)

    config = toml.loads(content)

    content = const.CONFIG_TEMPLATE.format_map(params)
    origin_config = toml.loads(content)
    merge_config(origin_config, origin_config['hidden'])

    config = merge_config(config, origin_config)
    for flag in ('manage_network', 'check_zenvfile_changes'):
        if not isinstance(config['run'][flag], bool):
            raise click.ClickException('run.{} must be a boolean'.format(flag))
    config['main']['config_hash'] = creation_hash

    # init logging
    if 'debug' in config['main'] and config['main']['debug']:
        loglevel = logging.DEBUG
    else:
        loglevel = logging.INFO
    logging.basicConfig(level=loglevel)
    return config


def find_file(path: str, fname: str):
    root_flag = path == '/'

    while True:
        fpath = os.path.join(path, fname)
        if os.path.isfile(fpath):
            return fpath

        path = os.path.dirname(path)
        if path == '/' and root_flag:
            return None

        root_flag = path == '/'


@contextmanager
def in_directory(path):
    pwd = os.getcwd()
    os.chdir(path)
    yield path
    os.chdir(pwd)


def merge_config(custom, origin):
    """
    Update `custom` config data from `origin` config data

    """
    if not isinstance(custom, dict):
        custom = {}

    for key, value in origin.items():
        if key not in custom:
            custom[key] = value
        elif isinstance(value, dict):
            custom[key] = merge_config(custom[key], value)
    return custom


def composit_environment(dotenv_env, zenvfile_env, blacklist):
    env = {}

    # low priority
    env.update({row.split('=', 1)[0]: row for row in dotenv_env})

    # medium priority
    env.update({row.split('=', 1)[0]: row for row in zenvfile_env})

    # high priority
    env.update({
        var: f'{var}={value}' for var, value in os.environ.items()
        if var not in blacklist
    })

    return list(env.values())


def build_docker_options(params):
    options = []
    for param, value in params.items():
        if value in ("true", "false"):
            option = [f'--{param}'] if value == "true" else []
        elif isinstance(value, list):
            option = []
            for val in value:
                option.extend([f'--{param}', val])
        else:
            option = [f'--{param}', value]
        options.extend(option)
    return options
