import json

import pytest

from zenv import core, utils


@pytest.mark.parametrize('source,expected', [
    ("KEY='value with spaces'\n", ['KEY=value with spaces']),
    ('KEY="value with spaces"\n', ['KEY=value with spaces']),
    ('KEY=value # comment\n', ['KEY=value']),
    ('# comment\n\nKEY=value\n', ['KEY=value']),
    ('export KEY=value\n', ['KEY=value']),
    ('EMPTY=\nBARE\n', ['EMPTY=', 'BARE']),
    ('KEY="one\ntwo"\n', ['KEY=one\ntwo']),
])
def test_load_dotenv_returns_parsed_values(tmp_path, source, expected):
    path = tmp_path / '.env'
    path.write_text(source)

    assert utils.load_dotenv(path) == expected


def test_quoted_json_reaches_docker_without_outer_quotes(tmp_path):
    payload = {
        'servers': 'paas',
        'topic': 'test-brand-bidding',
        'group_id': 'test-loyalty',
        'dlq': {'topic': 'test-brand-bidding-dlq'},
    }
    value = json.dumps(payload)
    path = tmp_path / '.env'
    path.write_text("TEST_CONSUMER__BRAND_BIDDING='{}'\n".format(value))

    command, options = core.get_command_with_options(
        ['env'], {}, {'env_file': str(path)},
    )

    assert command == ['env']
    row = 'TEST_CONSUMER__BRAND_BIDDING={}'.format(value)
    assert ['--env', row] in [options[i:i + 2] for i in range(len(options))]
    actual = next(
        option for option in options
        if option.startswith('TEST_CONSUMER__BRAND_BIDDING=')
    )
    assert json.loads(actual.split('=', 1)[1]) == payload
