"""Contract checks: example acceptance and invalid config rejection."""
import copy
import json
from pathlib import Path
import unittest

import yaml

from validate_config import StrictLoader, json_compatible, load_config, validate

ROOT = Path(__file__).resolve().parents[1]


class ConfigContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.schema = json.loads((ROOT / 'schema/config.schema.json').read_text())
        cls.example = load_config(ROOT / 'examples/config.yaml')

    def test_example(self):
        self.assertEqual(validate(self.example, self.schema), [])

    def test_lg_example(self):
        self.assertEqual(validate(load_config(ROOT / 'contrib/example-lg.yaml'), self.schema), [])

    def test_valid_commands(self):
        def device(config):
            return config['computers']['mac']['devices']['Desktop']

        for name, mutation in {
            'program name with no path': lambda c: device(c)['portal-match'].update(
                command='helper'),
            'quoted argument': lambda c: device(c)['portal-match'].update(
                command="helper --model 'LG ULTRAWIDE'"),
            'windows program': lambda c: device(c)['portal-match'].update(
                command=['C:\\bin\\helper.exe']),
            'command verifier': lambda c: device(c).update(
                verify={'driver': 'command', 'command': ['v'], 'timeout-ms': 90000}),
            'exec verifier': lambda c: device(c).update(verify={'driver': 'exec', 'command': 'v'}),
            'extension verifier': lambda c: device(c).update(
                verify={'driver': 'lab.http', 'options': {}}),
            'host list': lambda c: c['computers']['pc1']['endpoint'].update(
                host=['192.0.2.1', 'pc1.example']),
            'discovery with a host': lambda c: c['computers']['pc1']['endpoint'].update(
                discovery='mdns'),
            'discovery with no host': lambda c: c['computers']['pc1'].update(
                endpoint={'port': 24800, 'discovery': 'mdns'}),
        }.items():
            with self.subTest(name=name):
                config = copy.deepcopy(self.example)
                mutation(config)
                self.assertEqual(validate(config, self.schema), [])

    def test_invalid_configurations(self):
        def device(config):
            return config['computers']['mac']['devices']['Desktop']

        mutations = {
            'empty hint': lambda c: c['computers']['pc1'].update(hint=''),
            'no host and no discovery': lambda c: c['computers']['pc1'].update(
                endpoint={'port': 24800}),
            'empty host list': lambda c: c['computers']['pc1']['endpoint'].update(host=[]),
            'duplicate host': lambda c: c['computers']['pc1']['endpoint'].update(
                host=['a', 'a']),
            'unknown discovery': lambda c: c['computers']['pc1']['endpoint'].update(
                discovery='dns'),
            'non-string hint': lambda c: c['computers']['pc1'].update(hint=['x']),
            'long hint': lambda c: c['computers']['pc1'].update(hint='x' * 501),
            'non-boolean clipboard text': lambda c: c['computers']['pc1'].update(clipboard={'text': 'yes'}),
            'unknown clipboard key': lambda c: c['computers']['pc1'].update(clipboard={'image': True}),
            'zero scroll speed': lambda c: c['computers']['pc1'].update({'scroll-speed': 0}),
            'large scroll speed': lambda c: c['computers']['pc1'].update({'scroll-speed': 11}),
            'non-number scroll speed': lambda c: c['computers']['pc1'].update({'scroll-speed': 'fast'}),
            'unknown quirk': lambda c: c['computers']['pc1'].update({'disabled-quirks': ['absent']}),
            'duplicate quirk': lambda c: c['computers']['pc1'].update(
                {'disabled-quirks': ['kwin-button-scroll', 'kwin-button-scroll']}),
            'non-list quirks': lambda c: c['computers']['pc1'].update({'disabled-quirks': 'kwin-button-scroll'}),
            'version': lambda c: c.update(version=2),
            'unknown field': lambda c: device(c).update(portal_match={}),
            'duplicate device id': lambda c: c['computers']['mac']['devices'].update(
                Duplicate={'id': 'shared-lg'}),
            'missing computer': lambda c: device(c)['handoff'].update(
                absent=copy.deepcopy(device(c)['handoff']['pc1'])),
            'self handoff': lambda c: device(c)['handoff'].update(
                mac=copy.deepcopy(device(c)['handoff']['pc1'])),
            'missing connection': lambda c: c['computers']['pc1']['devices']['Monitor1'].update(
                id='different'),
            'empty steps': lambda c: device(c)['handoff']['pc1'].update(steps=[]),
            'bad timeout': lambda c: device(c)['portal-match'].update({'timeout-ms': 0}),
            'empty command list': lambda c: device(c)['portal-match'].update(command=[]),
            'empty command string': lambda c: device(c)['portal-match'].update(command=''),
            'blank command string': lambda c: device(c)['portal-match'].update(command='   '),
            'unbalanced quote': lambda c: device(c)['portal-match'].update(command="a 'b"),
            'relative program with a path': lambda c: device(c)['portal-match'].update(
                command=['bin/helper']),
            'relative cwd': lambda c: device(c)['portal-match'].update(cwd='work'),
            'relative search path': lambda c: c['computers']['mac'].update(path=['bin']),
            'empty search path': lambda c: c['computers']['mac'].update(path=[]),
            'unknown command field': lambda c: device(c)['portal-match'].update(shell=True),
            'options on a built-in driver': lambda c: device(c)['portal-match'].update(
                options={}),
            'command missing': lambda c: device(c)['portal-match'].pop('command'),
            'prompt out of verify': lambda c: device(c).update(
                {'portal-match': {'driver': 'prompt'}}),
            'exec for input-target': lambda c: device(c)['input-target'].update(driver='exec'),
            'prompt with a field': lambda c: device(c).update(
                verify={'driver': 'prompt', 'timeout-ms': 1000}),
            'verify on a route': lambda c: device(c)['handoff']['pc1'].update(
                verify={'driver': 'prompt'}),
            'unknown built-in driver': lambda c: device(c)['portal-match'].update(driver='shell'),
        }
        for name, mutation in mutations.items():
            with self.subTest(name=name):
                config = copy.deepcopy(self.example)
                mutation(config)
                self.assertTrue(validate(config, self.schema))

    def test_reject_ambiguous_yaml(self):
        for source in (
            'version: 1\nversion: 2',
            'x: &a [1]\ny: *a',
            'x: {<<: {a: 1}}',
            '1: value',
            'x: !custom value',
            'x: .nan',
            'x: 2026-10-27',
            'x: &a 1',
            'x: !!str value',
            'x: yes',
            'x: OFF',
            'x: y',
            'x: 010',
            'x: 1_000',
            'x: 0o17',
            '0o10: value',
            'x: 1:30',
            'x: 1e3',
            'x: 1.5e3',
            'x: 1.5e+3',
            '1.5e3: value',
            'x: =',
            'x: .inf',
            'a: 1\n---\nb: 2',
        ):
            with self.subTest(source=source):
                with self.assertRaises((ValueError, yaml.YAMLError)):
                    json_compatible(yaml.load(source, Loader=StrictLoader))

    def test_accept_json_compatible_yaml(self):
        for source, expected in (
            ("x: 'yes'", {'x': 'yes'}),
            ('x: 1.5', {'x': 1.5}),
            ('x: 0', {'x': 0}),
            ('x: -12', {'x': -12}),
            ('x: true', {'x': True}),
            ('x: ~', {'x': None}),
            ('x: hello world', {'x': 'hello world'}),
        ):
            with self.subTest(source=source):
                self.assertEqual(yaml.load(source, Loader=StrictLoader), expected)


if __name__ == '__main__':
    unittest.main()
