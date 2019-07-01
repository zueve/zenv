import os
import logging
from contextlib import contextmanager
import click
import toml
from . import const


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


def get_config(zenvfile=None):
    if not zenvfile:
        zenvfile = find_file(os.getcwd(), fname=const.DEFAULT_FILENAME)
    if not zenvfile:
        raise click.ClickException('Zenvfile don\'t find')

    config = toml.load(zenvfile)
    config = merge_config(config, const.CONFIG_DEFAULTS)
    config['zenvfile_path'] = zenvfile

    # init logging
    if 'debug' in config and config['debug']:
        loglevel = logging.DEBUG
    else:
        loglevel = logging.INFO
    logging.basicConfig(level=loglevel)
    return config


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


def composit_environment(static_env, blacklist):
    env = {}

    env = {
        var: value for var, value in os.environ.items()
        if var not in blacklist
    }
    if static_env:
        env.update(static_env)
    return env
