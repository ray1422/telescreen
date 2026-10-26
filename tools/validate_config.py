#!/usr/bin/env python3
"""Offline config validation only. Never executes configured operations."""
import argparse
import json
import math
import ntpath
import posixpath
import re
from pathlib import Path

import jsonschema
import yaml


# Plain scalars that YAML 1.1 and YAML 1.2 resolve differently (docs/config.md).
AMBIGUOUS_PLAIN = re.compile(r'''(?x)^(?:
    [yYnN] | yes|Yes|YES|no|No|NO|on|On|ON|off|Off|OFF
  | [0-9]{4}-[0-9]{1,2}-[0-9]{1,2}.*
  | [-+]?0[0-9]+ | [-+]?0b[01_]+ | [-+]?0o[0-7_]+ | [-+]?(?=[0-9._]*_)[0-9][0-9._]*
  | [-+]?[0-9][0-9_]*(?::[0-9_]+)+(?:\.[0-9_]*)?
  | [-+]?(?:[0-9][0-9_]*(?:\.[0-9_]*)?|\.[0-9][0-9_]*)[eE][-+]?[0-9]+
  | =
  | [-+]?\.(?:inf|Inf|INF) | \.(?:nan|NaN|NAN)
)$''')


class StrictLoader(yaml.SafeLoader):
    def compose_document(self):
        node = super().compose_document()
        if not self.check_event(yaml.StreamEndEvent):
            raise ValueError('only one YAML document is supported')
        return node

    def compose_node(self, parent, index):
        if self.check_event(yaml.AliasEvent):
            raise ValueError('YAML aliases are not supported')
        event = self.peek_event()
        if getattr(event, 'anchor', None) is not None:
            raise ValueError('YAML anchors are not supported')
        if isinstance(event, yaml.ScalarEvent):
            if event.tag is not None and not any(event.implicit):
                raise ValueError(f'YAML tags are not supported: {event.tag}')
            if event.style is None and AMBIGUOUS_PLAIN.match(event.value):
                raise ValueError(f'ambiguous plain scalar, quote it: {event.value}')
        elif isinstance(event, (yaml.MappingStartEvent, yaml.SequenceStartEvent)):
            if event.tag is not None and not event.implicit:
                raise ValueError(f'YAML tags are not supported: {event.tag}')
        return super().compose_node(parent, index)


def strict_mapping(loader, node):
    result = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=True)
        if not isinstance(key, str):
            raise ValueError('mapping keys must be strings')
        if key in result:
            raise ValueError(f'duplicate YAML key: {key}')
        result[key] = loader.construct_object(value_node, deep=True)
    return result


StrictLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, strict_mapping)


def json_compatible(value):
    if value is None or type(value) in (str, int, bool):
        return
    if type(value) is float and math.isfinite(value):
        return
    if type(value) is list:
        for child in value:
            json_compatible(child)
        return
    if type(value) is dict:
        for key, child in value.items():
            if not isinstance(key, str):
                raise ValueError('JSON keys must be strings')
            json_compatible(child)
        return
    raise ValueError(f'unsupported JSON value: {type(value).__name__}')


def load_config(path):
    config = yaml.load(path.read_text(encoding='utf-8'), Loader=StrictLoader)
    json_compatible(config)
    return config


def operations(device):
    for key in ('portal-match', 'input-target'):
        if key in device:
            yield device[key]
    for handoff in device.get('handoff', {}).values():
        yield from handoff['steps']
        if 'verify' in handoff:
            yield handoff['verify']


def validate(config, schema):
    jsonschema.Draft202012Validator.check_schema(schema)
    errors = [f'{list(e.absolute_path)}: {e.message}'
              for e in jsonschema.Draft202012Validator(schema).iter_errors(config)]
    if errors:
        return errors
    computers = config['computers']
    for computer_id, computer in computers.items():
        seen = set()
        for alias, device in computer['devices'].items():
            where = f'{computer_id}.devices.{alias}'
            device_id = device['id']
            if device_id in seen:
                errors.append(f'{where}: duplicate device id {device_id}')
            seen.add(device_id)
            for target in device.get('handoff', {}):
                if target == computer_id:
                    errors.append(f'{where}: cannot handoff to self')
                elif target not in computers:
                    errors.append(f'{where}: unknown target {target}')
                elif not any(d['id'] == device_id for d in computers[target]['devices'].values()):
                    errors.append(f'{where}: {target} has no device {device_id}')
            for operation in operations(device):
                if operation['driver'] != 'command':
                    continue
                options = operation['options']
                paths = [options['argv'][0]]
                if 'cwd' in options:
                    paths.append(options['cwd'])
                for path in paths:
                    if not (posixpath.isabs(path) or ntpath.isabs(path)):
                        errors.append(f'{where}: command path must be absolute: {path}')
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('config', type=Path)
    parser.add_argument('--schema', type=Path,
                        default=Path(__file__).resolve().parents[1] / 'schema/config.schema.json')
    args = parser.parse_args()
    try:
        errors = validate(load_config(args.config), json.loads(args.schema.read_text()))
    except (ValueError, OSError, yaml.YAMLError, jsonschema.SchemaError) as error:
        parser.exit(1, f'INVALID: {error}\n')
    if errors:
        parser.exit(1, '\n'.join(errors) + '\n')
    print(f'VALID: {args.config} (structure + references; drivers not executed)')


if __name__ == '__main__':
    main()
