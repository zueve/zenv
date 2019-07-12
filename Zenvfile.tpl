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

