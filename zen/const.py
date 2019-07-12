import os

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
__createuser__ = "useradd -m -r -u {uid} -g {gid} {id}"
"""
