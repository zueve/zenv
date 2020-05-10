import os
import sys
import click
import toml

from . import const, utils, core


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
@click.option('--template', '-t', default=None, help='Zenvfile template')
@click.option('--container-name', '-c', default=None, help='Container name')
def init(image, template, container_name):
    """Initialize Environment: create Zenvfile"""

    fpath = os.path.join(os.getcwd(), const.DEFAULT_FILENAME)
    if os.path.exists(fpath):
        raise click.ClickException(f'{fpath} already exist')

    if template:
        if not os.path.exists(template):
            raise click.ClickException(f'Tempate ({template}) not exist')

        with open(template, 'r') as zenvfile_template:
            zenv_template = zenvfile_template.read()
    else:
        zenv_template = const.INIT_TEMPLATE

    name = (
        container_name
        if container_name else os.path.basename(os.path.dirname(fpath))
    )
    image = image if image else const.DEFAULT_IMAGE

    config_str = zenv_template.format_map(utils.Default(
        id=const.CONTAINER_PREFIX,
        image=image,
        container_name=name or 'root',
        env_excludes='["' + '", "'.join(os.environ.keys()) + '"]'
    ))

    with open(fpath, 'w') as f:
        f.write(config_str)
    click.echo('Zenvfile created')


@cli.command(context_settings=dict(ignore_unknown_options=True),
             add_help_option=False)
@click.option('--zenvfile', default=None, help='Path to Zenvfile')
@click.argument('command', required=True, nargs=-1, type=click.UNPROCESSED)
def exec(zenvfile, command):
    """Call some command inside the container"""

    config = utils.get_config(zenvfile)
    exit_code = core.call(config, command)
    sys.exit(exit_code)


@cli.command()
@click.option('--zenvfile', default=None, help='Path to Zenvfile')
def info(zenvfile):
    """Show current container info"""

    config = utils.get_config(zenvfile)

    click.echo(f'Zenvfile: {config["main"]["zenvfilepath"]}')
    click.echo(f'Image: {config["main"]["image"]}')
    click.echo(f'Container: {config["main"]["name"]}')

    status = core.status(config['main']['name'])
    click.echo(f'Status: {status}')

    commands = [
        (alias, cmd)
        for alias, cmd in config['aliases'].items()
        if not alias.startswith('_')
    ]
    if commands:
        click.echo('Aliases:')
    for alias, cmd in commands:
        if 'description' in cmd:
            descr = cmd['description']
        else:
            descr = ' '.join(cmd['command'])
            if len(descr) > 80:
                descr = descr[: 74] + ' ...'

        click.echo(f' - {alias}: {descr}')


@cli.command()
@click.option('--zenvfile', default=None, help='Path to Zenvfile')
def stop(zenvfile):
    """Stop container"""

    config = utils.get_config(zenvfile)
    core.stop(config['main']['name'])


@cli.command()
@click.option('--zenvfile', default=None, help='Path to Zenvfile')
def rm(zenvfile):
    """Remove container (will stop the container, if need)"""

    config = utils.get_config(zenvfile)
    core.rm(config['main']['name'])


@cli.command(name='stop-all')
@click.option('--zenvfile', default=None, help='Path to Zenvfile')
@click.option('--exclude-current', '-e', 'exclude_current', is_flag=True,
              help='Exclude current contaiчner')
def stop_all(zenvfile, exclude_current):
    """Stop all zenv containers"""

    excludes = []

    if exclude_current:
        config = utils.get_config(zenvfile)
        excludes.append(config['main']['name'])

    core.stop_all(excludes)
