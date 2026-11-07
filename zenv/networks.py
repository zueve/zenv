"""Create named networks and remove only unused networks managed by Zenv."""

import json
import logging as logger
import subprocess

import click

from . import const


BUILTIN_NETWORKS = {'bridge', 'host', 'none', 'default'}


def _run(args):
    command = ['docker', *args]
    logger.debug(command)
    result = subprocess.run(command, capture_output=True)
    if result.returncode:
        raise click.ClickException(result.stderr.decode().strip())
    return result.stdout


def _inspect(network):
    try:
        return json.loads(_run(['network', 'inspect', '--', network]))[0]
    except click.ClickException:
        existing = _run([
            'network', 'ls', '--no-trunc', '--format', '{{json .}}',
        ])
        for line in existing.decode().splitlines():
            item = json.loads(line)
            if network in (item['Name'], item['ID']):
                raise
        return None


def configured_networks(spec):
    if spec is None or spec == '':
        return []
    names = [spec] if isinstance(spec, str) else spec
    if not isinstance(names, list) or any(
            not isinstance(name, str) for name in names):
        raise click.ClickException(
            'run.options.network must be a name or an array of names'
        )
    # Extended Docker network syntax and namespace modes are passed through.
    return list(dict.fromkeys(
        name for name in names
        if name and name not in BUILTIN_NETWORKS
        and not name.startswith('container:')
        and ',' not in name and '=' not in name
    ))


def ensure_networks(spec):
    """Reuse existing networks and return IDs created by this invocation."""
    created = []
    try:
        for name in configured_networks(spec):
            if _inspect(name) is not None:
                continue
            try:
                network_id = _run([
                    'network', 'create', '--label',
                    '{}=true'.format(const.MANAGED_NETWORK_LABEL), '--', name,
                ]).decode().strip()
            except click.ClickException:
                # Another process may have created the requested network.
                if _inspect(name) is None:
                    raise
            else:
                created.append(network_id)
    except Exception:
        for network_id in created:
            try:
                remove_if_unused(network_id)
            except click.ClickException as error:
                click.echo('Warning: {}'.format(error), err=True)
        raise
    return created


def _unused(network):
    if network.get('Containers'):
        return False
    # Stopped containers can retain a reference without an active endpoint.
    return not _run([
        'ps', '--all', '--filter', 'network={}'.format(network['Id']),
        '--format', '{{.ID}}',
    ]).strip()


def remove_if_unused(network_id):
    """Remove by ID only when the network is managed and has no references."""
    if network_id in BUILTIN_NETWORKS:
        return
    network = _inspect(network_id)
    if network is None or network['Name'] in BUILTIN_NETWORKS:
        return
    labels = network.get('Labels') or {}
    if labels.get(const.MANAGED_NETWORK_LABEL) != 'true':
        return
    if not _unused(network):
        return
    try:
        _run(['network', 'rm', '--', network['Id']])
    except click.ClickException:
        # Docker checks active endpoints atomically. A new user wins this race.
        remaining = _inspect(network['Id'])
        if remaining is not None and _unused(remaining):
            raise
