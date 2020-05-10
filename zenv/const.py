DEFAULT_FILENAME = 'Zenvfile'
DEFAULT_IMAGE = 'ubuntu:latest'

STATUS_RUNNING = 'Running'
STATUS_STOPED = 'Stoped'
STATUS_NOT_EXIST = 'Not_exist'

CONTAINER_PREFIX = 'zenv'
TEMPLATE_FILENAME = 'template.Zenvfile'

INIT_TEMPLATE = """# Zenvfile - state file for zenv-cli
# project https://github.com/zueve/zenv

[main]
version = 1
image = "{image}"
name = "{id}-{container_name}"
debug = false

[run]
command = ["__sleep__"]
init_commands = [["__create_group__"], ["__create_user__"]]

[run.options]
volume = ["{zenvfilepath}:{zenvfilepath}:rw"]
detach = "true"
publish = [] #  ports like "8888:8888"

[exec]
env_file = ""
env_excludes = {env_excludes}

[aliases]
__sleep__.command = ["sleep", "365d"]
__create_group__.command = ["groupadd", "-o", "--gid", "{gid}", "zenv"]
__create_user__.command = [
    "useradd", "-m", "-r", "-u", "{uid}", "-g", "{gid}", "{id}"
]
"""


CONFIG_TEMPLATE = INIT_TEMPLATE + """
[exec.options]
interactive = "true"
tty = "{tty}"
workdir = "{pwd}"
user = "{uid}:{gid}"

[hidden.main]
zenvfilepath = "{zenvfilepath}"

"""
