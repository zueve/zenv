import os
import sys
import click
import toml
from pathlib import Path

from . import const, utils


@click.group()
def cli():
    """
    ZENV(Zen-env): Containers manager for developer environment

    Usage:

    > zenv init -i centos:latest & ze cat /etc/redhat-release

    """
    pass


@cli.command()
@click.option('--image', '-i', default=const.DEFAULT_IMAGE,
              help='Docker Image')
@click.option('--container-name', '-c', default=None, help='Container name')
def init(image, container_name):
    """Initialize Environment: create Zenvfile"""

    fpath = os.path.join(os.getcwd(), const.DEFAULT_FILENAME)
    if os.path.exists(fpath):
        raise click.ClickException(f'{fpath} already exist')


    name = (
        container_name
        if container_name else os.path.basename(os.path.dirname(fpath))
    )
    image = image if image else const.DEFAULT_IMAGE

    class Default(dict):
        def __missing__(self, key):
            return f'{{key}}'

    config_str = const.CONFIG_TEMPLATE.format_map(
        id=const.CONTAINER_PREFIX,
        image=image,
        container_name=name or 'root'
    )

    with open(fpath, 'w') as f:
        f.write(config_str)
    click.echo('Zenvfile created')


@cli.command(context_settings=dict(ignore_unknown_options=True),
             add_help_option=False)
@click.option('--zenvfile', default=None, help='Path to Zenvfile')
@click.argument('command', required=True, nargs=-1, type=click.UNPROCESSED)
def exec(zenvfile, command):
    """Call some command inside the container"""

    config = core.get_config(zenvfile)
    print(config)


def get_config(zenvfile=None):
    if not zenvfile:
        zenvfile = utils.find_file(os.getcwd(), fname=const.DEFAULT_FILENAME)
    if not zenvfile:
        raise click.ClickException('Zenvfile don\'t find')

    content = Path(zenvfile).read_text()
    content = content.format(
        zenvfilepath=os.path.dirname(zenvfile),
        pwd=os.getcwd(),
        uid=os.getuid(),
        gid=os.getgid(),
        tty='true' if sys.stdin.isatty() else 'false'
    )


    config = toml.loads(content)
    config = merge_config(config, const.CONFIG_DEFAULTS)
    config['zenvfile_path'] = zenvfile

    # init logging
    if 'debug' in config['main'] and config['main']['debug']:
        loglevel = logging.DEBUG
    else:
        loglevel = logging.INFO
    logging.basicConfig(level=loglevel)
    return config
