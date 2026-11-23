"""Fingerprint declared container setup before invocation substitutions."""

import hashlib
import json
import os

from .utils import resolve_alias


def config_hash(config, project_directory):
    aliases = config['aliases']

    setup = {
        'project_directory': os.path.abspath(project_directory),
        'image': config['main']['image'],
        'command': resolve_alias(config['run']['command'], aliases)['command'],
        'options': config['run']['options'],
        'init_commands': [
            resolve_alias(command, aliases)
            for command in config['run']['init_commands']
        ],
    }
    content = json.dumps(setup, sort_keys=True, separators=(',', ':'))
    return hashlib.sha256(content.encode('utf-8')).hexdigest()
