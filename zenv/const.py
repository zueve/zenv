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
        'forvard_environment': [],
        'autoremove': False,
        'network': 'bridge',
        'init_commands': ['useradd -m -r -u `id -u` -g `id -gnr` `id -unr`'],
        'init_user_commands': [],
    },
    'environment': {
        'ZENVCONTAINER': '{container_name}',
    }
}
