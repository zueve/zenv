import os
import sys
import logging
from pathlib import Path
from contextlib import contextmanager
from functools import reduce
import click
import toml

from . import const


class Default(dict):
    def __missing__(self, key):
        return '{' + key + '}'


def load_dotenv(dotenvfile):
    try:
        with open(dotenvfile) as file:
            rows = file.read().splitlines()
    except FileNotFoundError:
        raise click.ClickException(f'Env file not found {dotenvfile}')

    environments = []
    for i, row in enumerate(rows):
        stripped_row = row.strip()
        if stripped_row:
            if '=' not in row:
                raise click.ClickException(
                    'Broken .env file. Each row should have `=`.'
                    f'Line {i}, val `{row}`'
                )
            environments.append(stripped_row)
    return environments


def get_config(zenvfile=None):
    if not zenvfile:
        zenvfile = find_file(os.getcwd(), fname=const.DEFAULT_FILENAME)
    if not zenvfile:
        raise click.ClickException('Zenvfile don\'t find. Make `zenv init`')

    params = Default(
        zenvfilepath=os.path.dirname(zenvfile),
        pwd=os.getcwd(),
        uid=os.getuid(),
        gid=os.getgid(),
        tty='true' if sys.stdin.isatty() else 'false',
        env_excludes='[]'
    )

    content = Path(zenvfile).read_text().format_map(params)

    config = toml.loads(content)

    content = const.CONFIG_TEMPLATE.format_map(params)
    origin_config = toml.loads(content)
    merge_config(origin_config, origin_config['hidden'])

    config = merge_config(config, origin_config)

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
