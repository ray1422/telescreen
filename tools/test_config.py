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

    def test_invalid_configurations(self):
        def device(config):
            return config['computers']['mac']['devices']['Desktop']

        mutations = {
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
            'empty argv': lambda c: device(c)['portal-match']['options'].update(argv=[]),
            'relative executable': lambda c: device(c)['portal-match']['options'].update(
                argv=['helper']),
            'unknown command option': lambda c: device(c)['portal-match']['options'].update(
                shell=True),
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
        ):
            with self.subTest(source=source):
                with self.assertRaises((ValueError, yaml.YAMLError)):
                    json_compatible(yaml.load(source, Loader=StrictLoader))


if __name__ == '__main__':
    unittest.main()
