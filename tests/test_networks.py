from types import SimpleNamespace
from unittest.mock import Mock

import pytest

from zenv import const, networks


@pytest.mark.parametrize('operation', [networks.create, networks.remove])
@pytest.mark.parametrize('spec', [None, []])
def test_absent_network_does_nothing(monkeypatch, operation, spec):
    process = Mock()
    monkeypatch.setattr(networks.subprocess, 'run', process)
    operation(spec)
    process.assert_not_called()


@pytest.mark.parametrize('operation', [networks.create, networks.remove])
@pytest.mark.parametrize('spec', [
    'project-dev', ['project-dev', 'shared'], 'bridge', 'host', 'none',
    'default', 'container:other', 'name=project-dev,alias=app',
])
def test_network_operations_are_direct_and_ignore_errors(
        monkeypatch, operation, spec):
    process = Mock(return_value=SimpleNamespace(returncode=1))
    monkeypatch.setattr(networks.subprocess, 'run', process)
    operation(spec)
    names = [spec] if isinstance(spec, str) else spec
    assert process.call_count == len(names)
    label = '{}=true'.format(const.MANAGED_NETWORK_LABEL)
    for invocation, name in zip(process.call_args_list, names):
        options = (
            ['create', '--label', label]
            if operation is networks.create else ['rm']
        )
        assert invocation.args == (
            ['docker', 'network', *options, '--', name],
        )
        assert invocation.kwargs == {
            'stdout': networks.subprocess.DEVNULL,
            'stderr': networks.subprocess.DEVNULL,
            'check': False,
        }
