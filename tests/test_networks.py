import json
from types import SimpleNamespace
from unittest.mock import Mock, call

import click
import pytest

from zenv import const, networks


NETWORK_ID = 'a' * 64


def result(code=0, value=None, stdout=b'', stderr=b'failed'):
    if value is not None:
        stdout = json.dumps(value).encode()
    return SimpleNamespace(returncode=code, stdout=stdout, stderr=stderr)


def network(managed=True, containers=None, name='project-dev'):
    return {
        'Id': NETWORK_ID,
        'Name': name,
        'Labels': {const.MANAGED_NETWORK_LABEL: 'true'} if managed else {},
        'Containers': containers or {},
    }


def process(monkeypatch, outcomes):
    command = Mock(side_effect=outcomes)
    monkeypatch.setattr(networks.subprocess, 'run', command)
    return command


@pytest.mark.parametrize('spec', [
    None, '', [], 'bridge', 'host', 'none', 'default', 'container:other',
    'name=project-dev,alias=app',
])
def test_unmanaged_network_modes_are_passed_through(monkeypatch, spec):
    command = process(monkeypatch, [])
    assert networks.ensure_networks(spec) == []
    command.assert_not_called()


@pytest.mark.parametrize('spec', [True, 42, [False]])
def test_invalid_network_option_has_a_clear_error(monkeypatch, spec):
    command = process(monkeypatch, [])
    with pytest.raises(click.ClickException, match='run.options.network'):
        networks.ensure_networks(spec)
    command.assert_not_called()


def test_existing_network_is_reused_without_changing_ownership(monkeypatch):
    command = process(monkeypatch, [result(value=[network(managed=False)])])
    assert networks.ensure_networks('project-dev') == []
    command.assert_called_once_with(
        ['docker', 'network', 'inspect', '--', 'project-dev'],
        capture_output=True,
    )


def test_missing_network_is_created_and_labeled(monkeypatch):
    command = process(monkeypatch, [
        result(1), result(), result(stdout=NETWORK_ID.encode()),
    ])
    assert networks.ensure_networks(['project-dev', 'project-dev']) == [
        NETWORK_ID,
    ]
    assert command.call_args_list[-1] == call([
        'docker', 'network', 'create', '--label',
        '{}=true'.format(const.MANAGED_NETWORK_LABEL), '--', 'project-dev',
    ], capture_output=True)


def test_create_race_reuses_winning_network(monkeypatch):
    command = process(monkeypatch, [
        result(1), result(), result(1), result(value=[network(False)]),
    ])
    assert networks.ensure_networks('project-dev') == []
    assert command.call_count == 4


def test_daemon_failure_never_creates_a_network(monkeypatch):
    command = process(monkeypatch, [
        result(1), result(1, stderr=b'daemon down'),
    ])
    with pytest.raises(click.ClickException, match='daemon down'):
        networks.ensure_networks('project-dev')
    assert command.call_count == 2


def test_inspection_error_is_not_treated_as_absence(monkeypatch):
    listed = json.dumps({'Name': 'project-dev', 'ID': NETWORK_ID}).encode()
    command = process(monkeypatch, [result(1), result(stdout=listed)])
    with pytest.raises(click.ClickException, match='failed'):
        networks.ensure_networks('project-dev')
    assert command.call_count == 2


def test_create_failure_cleans_only_newly_created_networks(monkeypatch):
    command = process(monkeypatch, [
        result(1), result(), result(stdout=NETWORK_ID.encode()),
        result(1), result(), result(1, stderr=b'cannot create'),
        result(1), result(),
        result(value=[network()]), result(), result(),
    ])
    with pytest.raises(click.ClickException, match='cannot create'):
        networks.ensure_networks(['first', 'second'])
    assert command.call_args_list[-1] == call(
        ['docker', 'network', 'rm', '--', NETWORK_ID], capture_output=True,
    )


@pytest.mark.parametrize('item', [
    network(managed=False),
    network(containers={'another': {'Name': 'database'}}),
    network(name='bridge'),
])
def test_external_builtin_and_connected_networks_are_retained(
        monkeypatch, item):
    command = process(monkeypatch, [result(value=[item])])
    networks.remove_if_unused(NETWORK_ID)
    assert command.call_count == 1


@pytest.mark.parametrize('name', ['bridge', 'host', 'none', 'default'])
def test_builtin_names_are_never_inspected_or_deleted(monkeypatch, name):
    command = process(monkeypatch, [])
    networks.remove_if_unused(name)
    command.assert_not_called()


def test_stopped_container_reference_keeps_network(monkeypatch):
    command = process(monkeypatch, [
        result(value=[network()]), result(stdout=b'stopped-container\n'),
    ])
    networks.remove_if_unused(NETWORK_ID)
    assert command.call_args_list[-1] == call([
        'docker', 'ps', '--all', '--filter', 'network={}'.format(NETWORK_ID),
        '--format', '{{.ID}}',
    ], capture_output=True)


def test_failed_usage_query_never_removes_network(monkeypatch):
    command = process(monkeypatch, [
        result(value=[network()]), result(1, stderr=b'cannot inspect users'),
    ])
    with pytest.raises(click.ClickException, match='cannot inspect users'):
        networks.remove_if_unused(NETWORK_ID)
    assert command.call_count == 2


def test_unused_managed_network_is_removed_by_id(monkeypatch):
    command = process(monkeypatch, [
        result(value=[network()]), result(), result(),
    ])
    networks.remove_if_unused('project-dev')
    assert command.call_args_list[-1] == call(
        ['docker', 'network', 'rm', '--', NETWORK_ID], capture_output=True,
    )


def test_remove_race_retains_a_newly_connected_network(monkeypatch):
    command = process(monkeypatch, [
        result(value=[network()]), result(), result(1),
        result(value=[network(containers={'new': {}})]),
    ])
    networks.remove_if_unused(NETWORK_ID)
    assert command.call_count == 4


def test_already_removed_network_is_success(monkeypatch):
    command = process(monkeypatch, [
        result(value=[network()]), result(), result(1), result(1), result(),
    ])
    networks.remove_if_unused(NETWORK_ID)
    assert command.call_count == 5


def test_unrelated_network_does_not_hide_inspect_failure(monkeypatch):
    listed = json.dumps({'Name': 'project-dev-other', 'ID': 'b' * 64}).encode()
    command = process(monkeypatch, [result(1), result(stdout=listed)])
    networks.remove_if_unused(NETWORK_ID)
    assert command.call_count == 2


def test_network_remove_failure_is_reported(monkeypatch):
    command = process(monkeypatch, [
        result(value=[network()]), result(),
        result(1, stderr=b'permission denied'),
        result(value=[network()]), result(),
    ])
    with pytest.raises(click.ClickException, match='permission denied'):
        networks.remove_if_unused(NETWORK_ID)
    assert command.call_count == 5
