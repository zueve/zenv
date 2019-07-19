import os
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

    config_str = const.CONFIG_TEMPLATE.format_map(utils.Default(
        id=const.CONTAINER_PREFIX,
        image=image,
        container_name=name or 'root',
        env_excludes='["' + '", "'.join(os.environ.keys()) + '"]'
    ))

    # hide some fields
    config = toml.loads(config_str)
    for row in const.HIDDEN_FIELDS:
        path = row.split('.')
        utils.delete_path_keys(config, path)

    with open(fpath, 'w') as f:
        toml.dump(config, f)
    click.echo('Zenvfile created')


@cli.command(context_settings=dict(ignore_unknown_options=True),
             add_help_option=False)
@click.option('--zenvfile', default=None, help='Path to Zenvfile')
@click.argument('command', required=True, nargs=-1, type=click.UNPROCESSED)
def exec(zenvfile, command):
    """Call some command inside the container"""

    config = utils.get_config(zenvfile)

    if config['exec']['env_file']:
        environments = utils.load_dotenv(config['exec']['env_file'])
    else:
        environments = []

    core.call(config, command, environments)


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
