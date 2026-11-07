from types import SimpleNamespace
from unittest.mock import Mock

import click
import pytest
from click.testing import CliRunner

from zenv import const, core, utils
from zenv.cli import cli


@pytest.fixture
def config(tmp_path):
    path = tmp_path / 'Zenvfile'
    path.write_text(
        '[main]\nimage = "example:latest"\nname = "zenv-test"\n'
        '[run]\ninit_commands = []\n'
        '[run.options]\nnetwork = "project-dev"\n'
    )
    return utils.get_config(str(path))


def missing_container(monkeypatch):
    monkeypatch.setattr(core, 'inspect_container', Mock(return_value=None))
    execute = Mock(return_value=23)
    monkeypatch.setattr(core, 'exec_', execute)
    return execute


def test_network_is_ready_before_container_creation(config, monkeypatch):
    assert config['run']['manage_network'] is True
    execute = missing_container(monkeypatch)
    events = []

    def ensure(spec):
        events.append(('network', spec))
        return ['created-network-id']

    def run(**kwargs):
        events.append(('run', kwargs['options']))
        return 0

    monkeypatch.setattr(core.networks, 'ensure_networks', ensure)
    monkeypatch.setattr(core, 'run', run)
    cleanup = Mock()
    monkeypatch.setattr(core.networks, 'remove_if_unused', cleanup)
    assert core.call(config, ['work']) == 23
    assert events[0] == ('network', 'project-dev')
    assert events[1][0] == 'run'
    options = events[1][1]
    assert options[options.index('--network') + 1] == 'project-dev'
    execute.assert_called_once()
    cleanup.assert_not_called()


def test_network_creation_failure_prevents_container_launch(
        config, monkeypatch):
    execute = missing_container(monkeypatch)
    ensure = Mock(side_effect=click.ClickException('cannot create network'))
    run = Mock()
    monkeypatch.setattr(core.networks, 'ensure_networks', ensure)
    monkeypatch.setattr(core, 'run', run)
    with pytest.raises(click.ClickException, match='cannot create network'):
        core.call(config, ['work'])
    run.assert_not_called()
    execute.assert_not_called()


def test_creation_without_configured_network_does_not_touch_networks(
        config, monkeypatch):
    missing_container(monkeypatch)
    config['run']['options'].pop('network')
    monkeypatch.setattr(core, 'run', Mock(return_value=0))
    network_process = Mock()
    monkeypatch.setattr(core.networks, '_run', network_process)
    assert core.call(config, ['work']) == 23
    network_process.assert_not_called()


@pytest.mark.parametrize('return_code', [0, 125])
def test_disabled_management_passes_network_without_managing_it(
        config, monkeypatch, return_code):
    execute = missing_container(monkeypatch)
    config['run']['manage_network'] = False
    run = Mock(return_value=return_code)
    ensure = Mock()
    cleanup = Mock()
    monkeypatch.setattr(core, 'run', run)
    monkeypatch.setattr(core.networks, 'ensure_networks', ensure)
    monkeypatch.setattr(core.networks, 'remove_if_unused', cleanup)
    assert core.call(config, ['work']) == (return_code or 23)
    options = run.call_args[1]['options']
    assert options[options.index('--network') + 1] == 'project-dev'
    ensure.assert_not_called()
    cleanup.assert_not_called()
    assert execute.call_count == int(return_code == 0)


@pytest.mark.parametrize('return_code', [125, OSError('cannot launch Docker')])
def test_failed_launch_cleans_only_new_networks(
        config, monkeypatch, return_code):
    execute = missing_container(monkeypatch)
    monkeypatch.setattr(
        core.networks, 'ensure_networks',
        Mock(return_value=['created-network-id']),
    )
    run = Mock(return_value=return_code)
    if isinstance(return_code, Exception):
        run.side_effect = return_code
    monkeypatch.setattr(core, 'run', run)
    cleanup = Mock()
    monkeypatch.setattr(core.networks, 'remove_if_unused', cleanup)
    if isinstance(return_code, Exception):
        with pytest.raises(OSError, match='cannot launch Docker'):
            core.call(config, ['work'])
    else:
        assert core.call(config, ['work']) == return_code
    cleanup.assert_called_once_with('created-network-id')
    execute.assert_not_called()


def test_rollback_failure_does_not_hide_launch_failure(
        config, monkeypatch, capsys):
    missing_container(monkeypatch)
    monkeypatch.setattr(
        core.networks, 'ensure_networks', Mock(return_value=['new-id'])
    )
    monkeypatch.setattr(core, 'run', Mock(return_value=125))
    monkeypatch.setattr(
        core.networks, 'remove_if_unused',
        Mock(side_effect=click.ClickException('cleanup failed')),
    )
    assert core.call(config, ['work']) == 125
    assert 'cleanup failed' in capsys.readouterr().err


@pytest.mark.parametrize('state', ['running', 'exited'])
def test_existing_container_does_not_create_newly_configured_network(
        config, monkeypatch, state):
    config['run']['check_zenvfile_changes'] = False
    monkeypatch.setattr(core, 'inspect_container', Mock(return_value={
        'Config': {'Labels': {const.CONFIG_HASH_LABEL: 'previous-settings'}},
        'State': {'Status': state},
    }))
    ensure = Mock()
    monkeypatch.setattr(core.networks, 'ensure_networks', ensure)
    monkeypatch.setattr(core, 'exec_', Mock(return_value=0))
    monkeypatch.setattr(
        core.subprocess, 'run',
        Mock(return_value=SimpleNamespace(returncode=0)),
    )
    assert core.call(config, ['work']) == 0
    ensure.assert_not_called()


def inspected_container(state='running'):
    return {
        'Id': 'original-container-id',
        'State': {'Status': state},
        'NetworkSettings': {'Networks': {
            'original-network': {'NetworkID': 'original-network-id'},
            'bridge': {'NetworkID': 'builtin-bridge-id'},
        }},
    }


def test_removal_cleans_only_explicitly_configured_network(
        monkeypatch):
    monkeypatch.setattr(
        core, 'inspect_container', Mock(return_value=inspected_container())
    )
    events = []

    def process(command):
        events.append(command)
        return SimpleNamespace(returncode=0)

    monkeypatch.setattr(core.subprocess, 'run', process)
    monkeypatch.setattr(
        core.networks, 'remove_if_unused',
        lambda network_id: events.append(('cleanup', network_id)),
    )
    core.rm('zenv-test', network='configured-network')
    assert events == [
        ['docker', 'stop', '--', 'original-container-id'],
        ['docker', 'rm', '--', 'original-container-id'],
        ('cleanup', 'configured-network'),
    ]


@pytest.mark.parametrize('network', [
    None, '', [], 'bridge', 'container:other',
])
def test_removal_without_named_setting_leaves_attached_networks(
        monkeypatch, network):
    monkeypatch.setattr(
        core, 'inspect_container', Mock(return_value=inspected_container())
    )
    process = Mock(return_value=SimpleNamespace(returncode=0))
    cleanup = Mock()
    monkeypatch.setattr(core.subprocess, 'run', process)
    monkeypatch.setattr(core.networks, 'remove_if_unused', cleanup)
    core.rm('zenv-test', network=network)
    assert process.call_count == 2
    cleanup.assert_not_called()


def test_disabled_management_never_removes_configured_network(monkeypatch):
    monkeypatch.setattr(
        core, 'inspect_container', Mock(return_value=inspected_container())
    )
    process = Mock(return_value=SimpleNamespace(returncode=0))
    cleanup = Mock()
    monkeypatch.setattr(core.subprocess, 'run', process)
    monkeypatch.setattr(core.networks, 'remove_if_unused', cleanup)
    core.rm('zenv-test', network='project-dev', manage_network=False)
    assert process.call_count == 2
    cleanup.assert_not_called()


def test_cli_passes_only_configured_network_to_removal(config, monkeypatch):
    remove = Mock()
    monkeypatch.setattr(core, 'rm', remove)
    path = '{}/Zenvfile'.format(config['main']['zenvfilepath'])
    runner = CliRunner()
    assert runner.invoke(cli, ['rm', '--zenvfile', path]).exit_code == 0
    remove.assert_called_once_with('zenv-test', 'project-dev', True)

    remove.reset_mock()
    with open(path) as stream:
        source = stream.read()
    with open(path, 'w') as stream:
        stream.write(source.replace('network = "project-dev"', ''))
    assert runner.invoke(cli, ['rm', '--zenvfile', path]).exit_code == 0
    remove.assert_called_once_with('zenv-test', None, True)

    remove.reset_mock()
    with open(path, 'w') as stream:
        stream.write(source.replace(
            '[run]\n', '[run]\nmanage_network = false\n'
        ))
    assert runner.invoke(cli, ['rm', '--zenvfile', path]).exit_code == 0
    remove.assert_called_once_with('zenv-test', 'project-dev', False)


@pytest.mark.parametrize('failure_at', ['stop', 'rm'])
def test_failed_container_removal_never_cleans_networks(
        monkeypatch, failure_at):
    monkeypatch.setattr(
        core, 'inspect_container', Mock(return_value=inspected_container())
    )

    def process(command):
        return SimpleNamespace(returncode=int(command[1] == failure_at))

    process_mock = Mock(side_effect=process)
    monkeypatch.setattr(core.subprocess, 'run', process_mock)
    cleanup = Mock()
    monkeypatch.setattr(core.networks, 'remove_if_unused', cleanup)
    with pytest.raises(click.ClickException):
        core.rm('zenv-test', network='project-dev')
    cleanup.assert_not_called()
    if failure_at == 'stop':
        assert process_mock.call_count == 1


def test_removing_an_absent_container_does_not_touch_networks(monkeypatch):
    monkeypatch.setattr(core, 'inspect_container', Mock(return_value=None))
    process = Mock()
    cleanup = Mock()
    monkeypatch.setattr(core.subprocess, 'run', process)
    monkeypatch.setattr(core.networks, 'remove_if_unused', cleanup)
    core.rm('zenv-test', network='project-dev')
    process.assert_not_called()
    cleanup.assert_not_called()
