"""Fingerprint declared container setup before invocation substitutions."""

import hashlib
import json
import os


def config_hash(config, project_directory):
    aliases = config['aliases']

    def expand(command):
        alias = aliases.get(command[0], {})
        return alias.get('command', list(command[:1])) + list(command[1:])

    setup = {
        'project_directory': os.path.abspath(project_directory),
        'image': config['main']['image'],
        'command': expand(config['run']['command']),
        'options': config['run']['options'],
        'init_commands': [
            {
                'command': expand(command),
                'exec': aliases.get(command[0], {}).get('exec', {}),
            }
            for command in config['run']['init_commands']
        ],
    }
    content = json.dumps(setup, sort_keys=True, separators=(',', ':'))
    return hashlib.sha256(content.encode('utf-8')).hexdigest()
