import subprocess
import os

import pytest


@pytest.fixture
def entrypoint():
    assert not os.path.exists('Zenvfile')
    subprocess.run('poetry run zenv init', shell=True)

    yield 'poetry run zenv'

    subprocess.run('poetry run zenv rm', shell=True)
    subprocess.run('rm Zenvfile', shell=True)


def test_init(entrypoint):
    assert subprocess.run(
        f'{entrypoint} init', shell=True).returncode == 1,  'allready exist'

    assert subprocess.run(
        f'{entrypoint} info', shell=True).returncode == 0

    res = subprocess.run(f'{entrypoint} exec ls', shell=True)
    assert res.returncode == 0

    res = subprocess.run(f'{entrypoint} exec __invalid_command__', shell=True)
    assert res.returncode > 0

    assert subprocess.run(
        f'{entrypoint} info', shell=True).returncode == 0

    assert subprocess.run(
        f'{entrypoint} stop', shell=True).returncode == 0

    assert subprocess.run(
        f'{entrypoint} exec ls', shell=True).returncode == 0

    assert subprocess.run(
        f'{entrypoint} stop-all', shell=True).returncode == 0

    assert subprocess.run(
        f'{entrypoint} info', shell=True).returncode == 0

    assert subprocess.run(
        f'{entrypoint} rm', shell=True).returncode == 0

    assert subprocess.run(
        f'{entrypoint} info', shell=True).returncode == 0
