DEFAULT_FILENAME = 'Zenvfile'
DEFAULT_IMAGE = 'ubuntu:latest'

STATUS_RUNNING = 'Running'
STATUS_STOPED = 'Stoped'
STATUS_NOT_EXIST = 'Not_exist'

CONTAINER_PREFIX = 'zenv'


CONFIG_TEMPLATE = """
[main]
image = "{image}"
name = "{id}-{container_name}"
zenvfilepath = "{zenvfilepath}"
debug = false

[run.options]
volume = ["{zenvfilepath}:{zenvfilepath}:rw"]
detach = "true"
publish = [] #  ports

[exec.options]
interactive = "true"
tty = "{tty}"
workdir = "{pwd}"
user = "{uid}:{gid}"

[run]
command = ["__sleep__"]
init_commands = [["__create_user__"]]

[exec]
env_file = ""
env_excludes = {env_excludes}

[commands]
__sleep__ = ["sleep", "365d"]
__create_user__ = [
    "useradd", "-m",  "-r", "-u", "{uid}", "-g", "{gid}", "{id}"
]
"""

HIDDEN_FIELDS = [
    'main.zenvfilepath',
    'exec.options'
]
