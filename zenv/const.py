import os

DEFAULT_FILENAME = 'Zenvfile'
DEFAULT_IMAGE = 'ubuntu:latest'

STATUS_RUNNING = 'Running'
STATUS_STOPED = 'Stoped'
STATUS_NOT_EXIST = 'Not_exist'

CONTAINER_PREFIX = 'zenv'

CONFIG_DEFAULTS = {
    'docker': {
        'image': '{image}',
        'container_name': f'{CONTAINER_PREFIX}-{{container_name}}',
    },
    'run': {
        'volumes': ['`pwd`:`pwd`:rw'],
        'ports': [],
        'blacklist_environment': list(os.environ.keys()),
        'autoremove': False,
        'network': 'bridge',
        'command': 'sleep infinity',
        'init_commands': ['useradd -m -r -u `id -u` -g `id -gnr` `id -unr`'],
        'init_user_commands': [],
    },
    'environment': {
        'ZENVCONTAINER': '{container_name}',
    }
}

CONFIG_TEMPLATE = """
[docker]
image = "{image}"
name = "zenv-{container_name}"
[run.options]
volume = ["{zenvfilepath}:{zenvfilepath}:rw"]
publish = [] #  ports
[exec.options]
interactive = true
tty = {tty}
workdir = "{pwd}"
user = "{uid}:{gid}"
[run]
command = "__sleep__"
init_commands = ["__createuser__"]
[exec]
env_file = ""
env_excludes = []
[aliases]
__sleep__ = "sleep 365d"
__createuser__ = "useradd -m -r -u {uid} -g {gid} zenv"
"""
