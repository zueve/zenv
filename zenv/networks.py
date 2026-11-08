"""Attempt configured network operations and let Docker handle conflicts."""

import logging as logger
import subprocess

from . import const


def _run(options, network):
    names = [network] if isinstance(network, str) else network or []
    for name in names:
        command = ['docker', 'network', *options, '--', name]
        logger.debug(command)
        subprocess.run(
            command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            check=False,
        )


def create(network):
    _run([
        'create', '--label', '{}=true'.format(const.MANAGED_NETWORK_LABEL),
    ], network)


def remove(network):
    _run(['rm'], network)
