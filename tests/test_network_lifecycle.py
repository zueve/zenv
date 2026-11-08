from types import SimpleNamespace
from unittest.mock import Mock

import click
import pytest
from click.testing import CliRunner

from zenv import core, utils
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


def test_network_creation_is_attempted_before_container_creation(
        config, monkeypatch):
    assert config['run']['manage_network'] is True
    execute = missing_container(monkeypatch)
    events = []
    monkeypatch.setattr(
        core.networks, 'create',
        lambda spec: events.append(('network', spec)),
    )

    def run(**kwargs):
        events.append(('run', kwargs['options']))
        return 0

    monkeypatch.setattr(core, 'run', run)
    assert core.call(config, ['work']) == 23
    assert events[0] == ('network', 'project-dev')
    assert events[1][0] == 'run'
    options = events[1][1]
    assert options[options.index('--network') + 1] == 'project-dev'
    execute.assert_called_once()


@pytest.mark.parametrize('return_code', [0, 1])
def test_network_create_result_does_not_prevent_container_launch(
        config, monkeypatch, return_code):
    execute = missing_container(monkeypatch)
    process = Mock(return_value=SimpleNamespace(returncode=return_code))
    monkeypatch.setattr(core.networks.subprocess, 'run', process)
    run = Mock(return_value=0)
    monkeypatch.setattr(core, 'run', run)
    assert core.call(config, ['work']) == 23
    process.assert_called_once()
    run.assert_called_once()
    execute.assert_called_once()


def test_creation_without_configured_network_does_not_touch_networks(
        config, monkeypatch):
    missing_container(monkeypatch)
    config['run']['options'].pop('network')
    monkeypatch.setattr(core, 'run', Mock(return_value=0))
    process = Mock()
    monkeypatch.setattr(core.networks.subprocess, 'run', process)
    assert core.call(config, ['work']) == 23
    process.assert_not_called()


@pytest.mark.parametrize('return_code', [0, 125])
def test_disabled_management_passes_network_without_managing_it(
        config, monkeypatch, return_code):
    execute = missing_container(monkeypatch)
    config['run']['manage_network'] = False
    run = Mock(return_value=return_code)
    create = Mock()
    monkeypatch.setattr(core, 'run', run)
    monkeypatch.setattr(core.networks, 'create', create)
    assert core.call(config, ['work']) == (return_code or 23)
    options = run.call_args[1]['options']
    assert options[options.index('--network') + 1] == 'project-dev'
    create.assert_not_called()
    assert execute.call_count == int(return_code == 0)


def test_failed_container_launch_does_not_roll_back_networks(
        config, monkeypatch):
    execute = missing_container(monkeypatch)
    process = Mock(return_value=SimpleNamespace(returncode=0))
    monkeypatch.setattr(core.networks.subprocess, 'run', process)
    monkeypatch.setattr(core, 'run', Mock(return_value=125))
    assert core.call(config, ['work']) == 125
    assert process.call_count == 1
    assert process.call_args.args[0][1:3] == ['network', 'create']
    execute.assert_not_called()


@pytest.mark.parametrize('state', ['running', 'exited'])
def test_existing_container_does_not_create_networks(
        config, monkeypatch, state):
    config['run']['check_zenvfile_changes'] = False
    monkeypatch.setattr(core, 'inspect_container', Mock(return_value={
        'State': {'Status': state},
    }))
    create = Mock()
    monkeypatch.setattr(core.networks, 'create', create)
    monkeypatch.setattr(core, 'exec_', Mock(return_value=0))
    monkeypatch.setattr(
        core.subprocess, 'run',
        Mock(return_value=SimpleNamespace(returncode=0)),
    )
    assert core.call(config, ['work']) == 0
    create.assert_not_called()


def inspected_container():
    return {
        'Id': 'original-container-id', 'State': {'Status': 'running'},
        'NetworkSettings': {'Networks': {'other-network': {}}},
    }


def test_removal_uses_only_configured_network_after_container_removal(
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
        core.networks, 'remove',
        lambda spec: events.append(('cleanup', spec)),
    )
    core.rm('zenv-test', network='configured-network')
    assert events == [
        ['docker', 'stop', '--', 'original-container-id'],
        ['docker', 'rm', '--', 'original-container-id'],
        ('cleanup', 'configured-network'),
    ]


@pytest.mark.parametrize('network', [None, []])
def test_removal_without_configured_network_leaves_attached_networks(
        monkeypatch, network):
    monkeypatch.setattr(
        core, 'inspect_container', Mock(return_value=inspected_container())
    )
    process = Mock(return_value=SimpleNamespace(returncode=0))
    monkeypatch.setattr(core.subprocess, 'run', process)
    core.rm('zenv-test', network=network)
    assert process.call_count == 2


def test_disabled_management_never_removes_configured_network(monkeypatch):
    monkeypatch.setattr(
        core, 'inspect_container', Mock(return_value=inspected_container())
    )
    monkeypatch.setattr(
        core.subprocess, 'run',
        Mock(return_value=SimpleNamespace(returncode=0)),
    )
    remove = Mock()
    monkeypatch.setattr(core.networks, 'remove', remove)
    core.rm('zenv-test', network='project-dev', manage_network=False)
    remove.assert_not_called()


def test_cli_passes_configured_network_and_management_flag(
        config, monkeypatch):
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
        stream.write(source.replace(
            '[run]\n', '[run]\nmanage_network = false\n'
        ))
    assert runner.invoke(cli, ['rm', '--zenvfile', path]).exit_code == 0
    remove.assert_called_once_with('zenv-test', 'project-dev', False)


@pytest.mark.parametrize('failure_at', ['stop', 'rm'])
def test_failed_container_removal_does_not_remove_networks(
        monkeypatch, failure_at):
    monkeypatch.setattr(
        core, 'inspect_container', Mock(return_value=inspected_container())
    )

    def process(command):
        return SimpleNamespace(returncode=int(command[1] == failure_at))

    monkeypatch.setattr(core.subprocess, 'run', Mock(side_effect=process))
    remove = Mock()
    monkeypatch.setattr(core.networks, 'remove', remove)
    with pytest.raises(click.ClickException):
        core.rm('zenv-test', network='project-dev')
    remove.assert_not_called()


def test_removing_absent_container_attempts_network_removal(monkeypatch):
    monkeypatch.setattr(core, 'inspect_container', Mock(return_value=None))
    process = Mock()
    remove = Mock()
    monkeypatch.setattr(core.subprocess, 'run', process)
    monkeypatch.setattr(core.networks, 'remove', remove)
    core.rm('zenv-test', network='project-dev')
    process.assert_not_called()
    remove.assert_called_once_with('project-dev')


def test_network_removal_error_does_not_fail_cli(config, monkeypatch):
    monkeypatch.setattr(core, 'inspect_container', Mock(return_value=None))
    monkeypatch.setattr(
        core.networks.subprocess, 'run',
        Mock(return_value=SimpleNamespace(returncode=1)),
    )
    path = '{}/Zenvfile'.format(config['main']['zenvfilepath'])
    output = CliRunner().invoke(cli, ['rm', '--zenvfile', path])
    assert output.exit_code == 0
    assert output.output == ''
