import io
import json
from copy import deepcopy
from types import SimpleNamespace
from unittest.mock import Mock, call

import click
import pytest
from click.testing import CliRunner

from zenv import const, core, fingerprint, utils
from zenv.cli import cli


@pytest.fixture
def project(tmp_path):
    path = tmp_path / 'Zenvfile'
    path.write_text(
        '[main]\nimage = "example:latest"\nname = "zenv-test"\n'
        '[run]\ninit_commands = []\n'
    )
    return path


@pytest.fixture
def declared():
    return {
        'main': {
            'image': 'example:latest', 'name': 'zenv-test', 'debug': False,
        },
        'run': {
            'command': ['sleep', '365d'],
            'options': {
                'volume': ['{zenvfilepath}:{zenvfilepath}:rw'],
                'publish': ['8000:8000'],
                'detach': 'true',
            },
            'init_commands': [['setup', '--first']],
        },
        'exec': {'options': {'user': '{uid}:{gid}', 'tty': '{tty}'}},
        'aliases': {
            'setup': {
                'command': ['provision', '{uid}', '{gid}', '{pwd}', '{tty}'],
                'exec': {'options': {'user': '0:0', 'env': ['MODE=setup']}},
                'description': 'Provision the project',
            },
            'unused': {'command': ['unused-command']},
        },
    }


def change(data, path, value):
    for key in path[:-1]:
        data = data[key]
    data[path[-1]] = value


@pytest.mark.parametrize('path,value', [
    (('main', 'image'), 'example:new'),
    (('run', 'command'), ['sleep', '30d']),
    (('run', 'options', 'publish'), ['9000:9000']),
    (('run', 'options', 'volume'), ['/other:/project:rw']),
    (('run', 'options', 'network'), 'different-network'),
    (('run', 'options', 'detach'), 'false'),
    (('run', 'init_commands'), [['setup', '--changed']]),
    (('aliases', 'setup', 'command'), ['different-provisioner']),
    (('aliases', 'setup', 'exec', 'options', 'user'), '123:456'),
    (('aliases', 'setup', 'exec', 'options', 'env'), ['MODE=other']),
])
def test_creation_settings_change_hash(declared, tmp_path, path, value):
    previous = fingerprint.config_hash(declared, str(tmp_path))
    change(declared, path, value)
    assert fingerprint.config_hash(declared, str(tmp_path)) != previous


def test_hash_is_canonical_and_ignores_unrelated_settings(declared, tmp_path):
    original = deepcopy(declared)
    previous = fingerprint.config_hash(declared, str(tmp_path))
    assert len(previous) == 64
    int(previous, 16)
    assert declared == original

    declared['main']['debug'] = True
    declared['exec'] = {'env_file': 'different.env', 'env_excludes': ['NEW']}
    declared['aliases']['unused']['command'] = ['other-command']
    declared['aliases']['setup']['description'] = 'New description'
    declared['run']['options'] = dict(
        reversed(list(declared['run']['options'].items()))
    )
    assert fingerprint.config_hash(declared, str(tmp_path)) == previous
    assert (
        fingerprint.config_hash(declared, str(tmp_path / 'other')) != previous
    )


def test_run_aliases_expand_without_hashing_unused_aliases(declared, tmp_path):
    previous = fingerprint.config_hash(declared, str(tmp_path))
    declared['run']['command'] = ['keep-running', '365d']
    declared['aliases']['keep-running'] = {'command': ['sleep']}
    assert fingerprint.config_hash(declared, str(tmp_path)) == previous
    declared['aliases']['keep-running']['command'] = ['different-sleep']
    assert fingerprint.config_hash(declared, str(tmp_path)) != previous


def test_comments_key_order_and_explicit_defaults_do_not_change_hash(project):
    previous = utils.get_config(str(project))['main']['config_hash']
    project.write_text(
        '# Same settings in a different order\n'
        '[run]\ninit_commands = []\ncommand = ["__sleep__"]\n'
        '[run.options]\npublish = []\ndetach = "true"\n'
        'volume = ["{zenvfilepath}:{zenvfilepath}:rw"]\n'
        '[main]\nname = "zenv-test"\nimage = "example:latest"\n'
    )
    assert utils.get_config(str(project))['main']['config_hash'] == previous


@pytest.mark.parametrize('flag', ['manage_network', 'check_zenvfile_changes'])
def test_policy_switch_does_not_require_recreation(project, flag):
    original = utils.get_config(str(project))
    assert original['run'][flag] is True
    previous = original['main']['config_hash']
    source = project.read_text()
    project.write_text(source.replace(
        '[run]\n', '[run]\n{} = false\n'.format(flag)
    ))
    config = utils.get_config(str(project))
    assert config['run'][flag] is False
    assert config['main']['config_hash'] == previous


@pytest.mark.parametrize('value', ['"false"', '0', '[]'])
@pytest.mark.parametrize('flag', ['manage_network', 'check_zenvfile_changes'])
def test_policy_requires_a_toml_boolean(project, value, flag):
    source = project.read_text()
    project.write_text(source.replace(
        '[run]\n', '[run]\n{} = {}\n'.format(flag, value)
    ))
    with pytest.raises(click.ClickException, match=flag):
        utils.get_config(str(project))


def test_inline_tables_and_escaped_placeholders_keep_their_meaning(project):
    source = (
        '[main]\nimage = "example:latest"\nname = "zenv-test"\n'
        '[run]\ninit_commands = []\ncommand = ["work"]\n'
        '[aliases]\nwork = {{ command = ["echo", "{uid}"] }}\n'
    )
    project.write_text(source)
    active = utils.get_config(str(project))
    assert active['aliases']['work']['command'] == [
        'echo', str(utils.os.getuid())
    ]
    project.write_text(source.replace('"{uid}"', '"{{uid}}"'))
    literal = utils.get_config(str(project))
    assert literal['aliases']['work']['command'] == ['echo', '{uid}']
    assert active['main']['config_hash'] != literal['main']['config_hash']


def test_invocation_values_do_not_change_hash(project, monkeypatch):
    source = project.read_text() + (
        '[aliases.setup]\n'
        'command = ["provision", "{uid}", "{gid}", "{pwd}", "{tty}"]\n'
    )
    source = source.replace(
        'init_commands = []', 'init_commands = [["setup"]]'
    )
    project.write_text(source)
    monkeypatch.chdir(project.parent)
    monkeypatch.setattr(utils.os, 'getuid', lambda: 501)
    monkeypatch.setattr(utils.os, 'getgid', lambda: 20)
    monkeypatch.setattr(utils.sys.stdin, 'isatty', lambda: False)
    before = utils.get_config()

    child = project.parent / 'nested'
    child.mkdir()
    monkeypatch.chdir(child)
    monkeypatch.setattr(utils.os, 'getuid', lambda: 0)
    monkeypatch.setattr(utils.os, 'getgid', lambda: 0)
    monkeypatch.setattr(utils.sys.stdin, 'isatty', lambda: True)
    monkeypatch.setenv('ZENV_HASH_TEST_VALUE', 'host environment changed')
    after = utils.get_config()
    assert before['main']['config_hash'] == after['main']['config_hash']
    assert before['exec']['options']['user'] == '501:20'
    assert after['exec']['options']['user'] == '0:0'
    assert after['exec']['options']['tty'] == 'true'
    assert after['exec']['options']['workdir'] == str(child)
    assert utils.get_config('../Zenvfile')['main']['config_hash'] == (
        utils.get_config(str(project))['main']['config_hash']
    )

    project.write_text(source.replace('"{uid}"', '"1234"'))
    assert (
        utils.get_config()['main']['config_hash']
        != before['main']['config_hash']
    )


def container(config, state='running', saved_hash=True):
    labels = None if saved_hash is None else {
        const.CONFIG_HASH_LABEL: (
            config['main']['config_hash'] if saved_hash is True else saved_hash
        ),
    }
    return {
        'Id': 'container-id',
        'Config': {'Labels': labels},
        'State': {'Status': state},
    }


def runtime(monkeypatch, config, existing):
    inspect = Mock(return_value=existing)
    run = Mock(return_value=0)
    execute = Mock(return_value=27)
    process = Mock(return_value=SimpleNamespace(returncode=0))
    monkeypatch.setattr(core, 'inspect_container', inspect)
    monkeypatch.setattr(core, 'run', run)
    monkeypatch.setattr(core, 'exec_', execute)
    monkeypatch.setattr(core.subprocess, 'run', process)
    return inspect, run, execute, process


@pytest.mark.parametrize('state', ['running', 'exited', 'created'])
def test_matching_container_runs_commands(project, monkeypatch, state, capsys):
    config = utils.get_config(str(project))
    inspect, run, execute, process = runtime(
        monkeypatch, config, container(config, state)
    )
    assert core.call(config, ['work', 'a b']) == 27
    inspect.assert_called_once_with('zenv-test')
    run.assert_not_called()
    assert execute.call_args[0][:2] == ('zenv-test', ['work', 'a b'])
    if state == 'running':
        process.assert_not_called()
    else:
        process.assert_called_once_with(['docker', 'start', 'zenv-test'])
    assert capsys.readouterr().err == ''


@pytest.mark.parametrize('state', ['running', 'exited', 'created'])
@pytest.mark.parametrize('saved_hash', ['outdated', None])
def test_declined_recreation_warns_and_continues(
        project, monkeypatch, state, saved_hash, capsys):
    config = utils.get_config(str(project))
    _, run, execute, process = runtime(
        monkeypatch, config, container(config, state, saved_hash)
    )
    monkeypatch.setattr(core.sys.stdin, 'isatty', lambda: True)
    confirm = Mock(return_value=False)
    monkeypatch.setattr(core.click, 'confirm', confirm)
    assert core.call(config, ['work']) == 27
    confirm.assert_called_once_with(
        'Recreate the container with the updated settings?',
        default=False, err=True,
    )
    output = capsys.readouterr()
    assert output.out == ''
    assert 'Warning:' in output.err
    run.assert_not_called()
    execute.assert_called_once()
    if state == 'running':
        process.assert_not_called()
    else:
        process.assert_called_once_with(['docker', 'start', 'zenv-test'])


@pytest.mark.parametrize('labels', [
    [], ['team=tools', 'purpose=dev'], 'team=tools',
])
@pytest.mark.parametrize('check_zenvfile_changes', [True, False])
def test_creation_saves_hash_and_preserves_labels(
        project, monkeypatch, labels, check_zenvfile_changes):
    config = utils.get_config(str(project))
    config['run']['check_zenvfile_changes'] = check_zenvfile_changes
    config['run']['options']['label'] = labels
    original = deepcopy(config)
    _, run, execute, process = runtime(monkeypatch, config, None)
    assert core.call(config, ['work']) == 27
    options = run.call_args[1]['options']
    saved = [options[index + 1] for index, value in enumerate(options)
             if value == '--label']
    expected = labels if isinstance(labels, list) else [labels]
    assert saved == expected + [
        '{}={}'.format(const.CONFIG_HASH_LABEL, config['main']['config_hash'])
    ]
    assert config == original
    execute.assert_called_once()
    process.assert_not_called()


def test_creation_and_start_failures_prevent_execution(project, monkeypatch):
    config = utils.get_config(str(project))
    _, run, execute, _ = runtime(monkeypatch, config, None)
    run.return_value = 7
    assert core.call(config, ['work']) == 7
    execute.assert_not_called()

    _, _, execute, process = runtime(
        monkeypatch, config, container(config, 'exited')
    )
    process.return_value.returncode = 8
    assert core.call(config, ['work']) == 8
    execute.assert_not_called()


def result(code=0, stdout=b'', stderr=b''):
    return SimpleNamespace(returncode=code, stdout=stdout, stderr=stderr)


def test_inspect_uses_exact_target_and_structured_state(monkeypatch):
    expected = {'State': {'Status': 'exited'}, 'Config': {'Labels': {}}}
    process = Mock(return_value=result(stdout=json.dumps([expected]).encode()))
    monkeypatch.setattr(core.subprocess, 'run', process)
    assert core.inspect_container('zenv-test') == expected
    process.assert_called_once_with(
        ['docker', 'container', 'inspect', '--', 'zenv-test'],
        capture_output=True,
    )


def test_inspect_absence_requires_a_successful_exact_name_query(monkeypatch):
    process = Mock(side_effect=[
        result(1, stderr=b'not found'),
        result(stdout=b'zenv-test-other\n'),
    ])
    monkeypatch.setattr(core.subprocess, 'run', process)
    assert core.inspect_container('zenv-test') is None
    assert process.call_count == 2


@pytest.mark.parametrize('names', [
    result(1, stderr=b'cannot connect to daemon'),
    result(stdout=b'zenv-test\n'),
])
def test_inspect_errors_are_not_missing_containers(monkeypatch, names):
    process = Mock(side_effect=[result(1, stderr=b'inspect failed'), names])
    monkeypatch.setattr(core.subprocess, 'run', process)
    with pytest.raises(click.ClickException):
        core.inspect_container('zenv-test')


class TerminalInput(io.BytesIO):
    def isatty(self):
        return True


@pytest.mark.parametrize('exit_code', [0, 27])
@pytest.mark.parametrize('state', ['running', 'exited', 'created'])
@pytest.mark.parametrize('saved_hash', ['outdated', None])
def test_confirmed_recreation_initializes_and_retries_original_command(
        project, monkeypatch, exit_code, state, saved_hash):
    project.write_text(
        project.read_text().replace(
            'init_commands = []', 'init_commands = [["setup"]]'
        ) + '[aliases]\nwork.command = ["tool", "alias arg"]\n'
    )
    config = utils.get_config(str(project))
    existing = container(config, state, saved_hash)
    inspect, run, execute, process = runtime(
        monkeypatch, config, existing
    )
    inspect.side_effect = [existing, existing, None]
    execute.side_effect = [0, exit_code]
    output = CliRunner().invoke(cli, [
        'exec', '--zenvfile', str(project), 'work', 'a b', '--flag',
    ], input=TerminalInput(b'y\n'))
    assert output.exit_code == exit_code
    assert 'Warning:' in output.output
    assert 'Recreate the container with the updated settings?' in output.output
    assert 'Traceback' not in output.output
    assert inspect.call_args_list == [call('zenv-test')] * 3
    run.assert_called_once()
    assert execute.call_count == 2
    assert execute.call_args_list[0][0][:2] == ('zenv-test', ['setup'])
    assert execute.call_args_list[1][0][:2] == (
        'zenv-test', ['tool', 'alias arg', 'a b', '--flag']
    )
    expected = [call(['docker', 'rm', '--', 'container-id'])]
    if state == 'running':
        expected.insert(0, call(['docker', 'stop', '--', 'container-id']))
    assert process.call_args_list == expected


@pytest.mark.parametrize('answer', [b'n\n', b'\n'])
@pytest.mark.parametrize('state', ['running', 'exited', 'created'])
def test_declined_recreation_executes_without_removal(
        project, monkeypatch, answer, state):
    config = utils.get_config(str(project))
    _, run, execute, process = runtime(
        monkeypatch, config, container(config, state, 'outdated')
    )
    remove = Mock()
    monkeypatch.setattr(core, 'rm', remove)
    output = CliRunner().invoke(cli, [
        'exec', '--zenvfile', str(project), 'work',
    ], input=TerminalInput(answer))
    assert output.exit_code == 27
    assert 'Recreate the container with the updated settings?' in output.output
    remove.assert_not_called()
    run.assert_not_called()
    execute.assert_called_once()
    if state == 'running':
        process.assert_not_called()
    else:
        process.assert_called_once_with(['docker', 'start', 'zenv-test'])


@pytest.mark.parametrize('manage_network', [True, False])
def test_failed_recreation_does_not_start_or_execute(
        project, monkeypatch, manage_network):
    project.write_text(
        project.read_text().replace(
            'init_commands = []',
            'init_commands = []\nmanage_network = {}'.format(
                str(manage_network).lower()
            ),
        ) + '[run.options]\nnetwork = "project-dev"\n'
    )
    config = utils.get_config(str(project))
    _, run, execute, process = runtime(
        monkeypatch, config, container(config, 'running', 'outdated')
    )
    remove = Mock(side_effect=click.ClickException('Cannot remove container'))
    monkeypatch.setattr(core, 'rm', remove)
    prepare = Mock()
    monkeypatch.setattr(core, 'get_command_with_options', prepare)
    output = CliRunner().invoke(cli, [
        'exec', '--zenvfile', str(project), 'work',
    ], input=TerminalInput(b'y\n'))
    assert output.exit_code == 1
    assert 'Cannot remove container' in output.output
    remove.assert_called_once_with('zenv-test', 'project-dev', manage_network)
    prepare.assert_not_called()
    run.assert_not_called()
    execute.assert_not_called()
    process.assert_not_called()


def test_failed_creation_after_removal_does_not_execute(project, monkeypatch):
    config = utils.get_config(str(project))
    existing = container(config, saved_hash='outdated')
    inspect, run, execute, _ = runtime(monkeypatch, config, existing)
    inspect.side_effect = [existing, None]
    remove = Mock()
    monkeypatch.setattr(core, 'rm', remove)
    monkeypatch.setattr(core.sys.stdin, 'isatty', lambda: True)
    monkeypatch.setattr(core.click, 'confirm', Mock(return_value=True))
    run.return_value = 7
    assert core.call(config, ['work']) == 7
    remove.assert_called_once_with('zenv-test', None, True)
    run.assert_called_once()
    execute.assert_not_called()


def test_noninteractive_mismatch_does_not_consume_stdin(project, monkeypatch):
    config = utils.get_config(str(project))
    _, run, execute, process = runtime(
        monkeypatch, config, container(config, 'exited', 'outdated')
    )
    stdin = io.StringIO('command input\n')
    monkeypatch.setattr(core.sys, 'stdin', stdin)
    confirm = Mock()
    monkeypatch.setattr(core.click, 'confirm', confirm)
    with pytest.raises(
            click.ClickException, match='check_zenvfile_changes = false'):
        core.call(config, ['work'])
    assert stdin.read() == 'command input\n'
    confirm.assert_not_called()
    run.assert_not_called()
    execute.assert_not_called()
    process.assert_not_called()


@pytest.mark.parametrize('state', ['running', 'exited'])
@pytest.mark.parametrize('saved_hash', ['outdated', None])
def test_disabled_fingerprint_check_skips_warning_and_confirmation(
        project, monkeypatch, state, saved_hash, capsys):
    config = utils.get_config(str(project))
    config['run']['check_zenvfile_changes'] = False
    _, run, execute, process = runtime(
        monkeypatch, config, container(config, state, saved_hash)
    )
    confirm = Mock()
    monkeypatch.setattr(core.click, 'confirm', confirm)
    monkeypatch.setattr(core.sys, 'stdin', io.StringIO('command input\n'))
    assert core.call(config, ['work']) == 27
    assert core.sys.stdin.read() == 'command input\n'
    assert capsys.readouterr() == ('', '')
    confirm.assert_not_called()
    run.assert_not_called()
    execute.assert_called_once()
    if state == 'running':
        process.assert_not_called()
    else:
        process.assert_called_once_with(['docker', 'start', 'zenv-test'])


def test_info_and_removal_work_despite_mismatching_hash(project, monkeypatch):
    config = utils.get_config(str(project))
    _, _, _, process = runtime(
        monkeypatch, config, container(config, 'exited', 'outdated')
    )
    runner = CliRunner()
    output = runner.invoke(cli, ['info', '--zenvfile', str(project)])
    assert output.exit_code == 0
    assert 'Status: {}'.format(const.STATUS_STOPED) in output.output
    output = runner.invoke(cli, ['rm', '--zenvfile', str(project)])
    assert output.exit_code == 0
    process.assert_called_once_with(['docker', 'rm', '--', 'container-id'])


def test_command_preparation_does_not_mutate_config(monkeypatch):
    aliases = {'work': {
        'command': ['tool'],
        'exec': {'options': {'env': ['FROM_ALIAS=one']}},
    }}
    settings = {'options': {'user': '123:456', 'env': ['FROM_BASE=one']}}
    before = deepcopy((aliases, settings))
    monkeypatch.setenv('ZENV_HASH_TEST_VALUE', 'host value')
    command, options = core.get_command_with_options(
        ['work'], aliases, settings
    )
    assert command == ['tool']
    assert 'ZENV_HASH_TEST_VALUE=host value' in options
    assert (aliases, settings) == before
