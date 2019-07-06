import os
import sys
import click
import toml

from zenv import core, const, utils


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
@click.option('--update', '-u', 'is_update', is_flag=True, default=False,
              help='Merge current Zenvfile with defaults')
def init(image, container_name, is_update):
    """Initialize Invironment: create Zenvfile"""

    fpath = os.path.join(os.getcwd(), const.DEFAULT_FILENAME)
    if is_update:
        if not os.path.exists(fpath):
            raise click.ClickException(f'{fpath} does\'t exist')
        config_template = utils.get_config(fpath)
        config_template['docker'] = const.CONFIG_DEFAULTS['docker']
        del config_template['zenvfile_path']
    else:
        if os.path.exists(fpath):
            raise click.ClickException(f'{fpath} already exist')
        config_template = const.CONFIG_DEFAULTS

    if not container_name:
        container_name = os.path.basename(os.path.dirname(fpath)) or 'root'

    content = toml.dumps(config_template).format(
        image=image,
        container_name=container_name
    )

    with open(fpath, 'w') as f:
        f.write(content)
    click.echo('Zenv created')


@cli.command(context_settings=dict(ignore_unknown_options=True),
             add_help_option=False)
@click.option('--zenvfile', default=None, help='Path to zenvfile')
@click.argument('command', required=True, nargs=-1, type=click.UNPROCESSED)
def exec(zenvfile, command):
    """Call some command inside container"""

    config = utils.get_config(zenvfile)
    tty = sys.stdin.isatty()
    core.call(config, '"' + '" "'.join(command) + '"', tty)


@cli.command()
@click.option('--zenvfile', default=None, help='Path to zenvfile')
def info(zenvfile):
    """Show current container info"""

    config = utils.get_config(zenvfile)

    click.echo(f'Zenvfile: {config["zenvfile_path"]}')
    click.echo(f'Image: {config["docker"]["image"]}')
    click.echo(f'Container: {config["docker"]["container_name"]}')

    status = core.status(config['docker']['container_name'])
    click.echo(f'Status: {status}')


@cli.command()
@click.option('--zenvfile', default=None, help='Path to zenvfile')
def stop(zenvfile):
    """Stop container"""

    config = utils.get_config(zenvfile)
    core.stop(config['docker']['container_name'])


@cli.command()
@click.option('--zenvfile', default=None, help='Path to zenvfile')
def rm(zenvfile):
    """Remove container (will stop container, if nedd)"""

    config = utils.get_config(zenvfile)
    core.rm(config['docker']['container_name'])


@cli.command(name='stop-all')
@click.option('--zenvfile', default=None, help='Path to zenvfile')
@click.option('--exclude-current', '-e', 'exclude_current', is_flag=True,
              help='Exclude current container')
def stop_all(zenvfile, exclude_current):
    """Stop all zenv containers"""

    excludes = []

    if exclude_current:
        config = utils.get_config(zenvfile)
        excludes.append(config['docker']['container_name'])

    core.stop_all(excludes)


if __name__ == '__main__':
    cli()
