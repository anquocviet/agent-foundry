#!/usr/bin/env python3
"""Development prerequisite: jsonschema; validates semantic boundaries, not wording."""
import copy
import json
from pathlib import Path
import sys
from jsonschema import Draft202012Validator

root = Path(__file__).resolve().parents[2]
for path in (root / 'schemas').glob('*.json'):
    Draft202012Validator.check_schema(json.loads(path.read_text()))

validator = Draft202012Validator(json.loads((root / 'schemas/config.json').read_text()))
valid = {'schema': 'agentfoundry.config/v1', 'adapters': ['agents-md'], 'addons': [],
         'paths': {'docs': 'docs/agentfoundry', 'skills': '.agents/skills'},
         'updates': {'channel': 'stable', 'automatic': False}, 'telemetry': {'enabled': False}}
validator.validate(valid)
for key, value in [('schema', 'agentfoundry.config/v99'), ('adapters', ['agents-md', 'agents-md']),
                   ('addons', ['untrusted-code'])]:
    bad = copy.deepcopy(valid)
    bad[key] = value
    assert list(validator.iter_errors(bad)), key
bad = copy.deepcopy(valid)
bad['updates']['endpoint'] = 'https://untrusted.example'
assert list(validator.iter_errors(bad))
bad = copy.deepcopy(valid)
bad['telemetry']['enabled'] = True
assert list(validator.iter_errors(bad))

plan_validator = Draft202012Validator(json.loads((root / 'schemas/plan.json').read_text()))
plan = {'schema': 'agentfoundry.plan/v1', 'root': '/fixture', 'payload_digest': 'a' * 64,
        'root_identity': None,
        'snapshot': None,
        'operations': [{'target': 'AGENTS.md', 'before': None, 'after': '6162', 'mode': 420}],
        'choices': {key: False for key in ('merge', 'replace', 'refresh', 'wisdom', 'claude')}}
plan_validator.validate(plan)
for value in ['abc', 'gg', [1, 2], {'bytes': '00'}]:
    bad = copy.deepcopy(plan)
    bad['operations'][0]['after'] = value
    assert list(plan_validator.iter_errors(bad)), value
distribution_validator = Draft202012Validator(json.loads((root / 'schemas/distribution.json').read_text()))
distribution = json.loads((root / 'payload/manifest.json').read_text())
distribution_validator.validate(distribution)
for key, value in [('logical_id', ''), ('target', ''), ('addon', True), ('adapter', 'unknown')]:
    bad = copy.deepcopy(distribution)
    bad['entries'][0][key] = value
    assert list(distribution_validator.iter_errors(bad)), key
installation_validator = Draft202012Validator(json.loads((root / 'schemas/installation.json').read_text()))
installation = {'schema': 'agentfoundry.installation/v1', 'generation': 1, 'version': '0.1.0-dev.1',
                'source': 'fixture', 'payload_digest': 'a' * 64, 'binary_digest': 'b' * 64, 'entries': []}
installation_validator.validate(installation)
for field in ('version', 'source'):
    bad = copy.deepcopy(installation)
    bad[field] = ''
    assert list(installation_validator.iter_errors(bad)), field
snapshot_validator = Draft202012Validator(json.loads((root / 'schemas/snapshot.json').read_text()))
snapshot = {'schema': 'agentfoundry.snapshot/v1', 'root_identity': '/fixture', 'generation': 1,
            'config_digest': None, 'state_digest': None,
            'entries': [{'path': 'missing.md', 'presence': {'kind': 'absent'}},
                        {'path': 'local.md', 'presence': {'kind': 'file', 'digest': 'a' * 64, 'mode': 420}}]}
snapshot['entries'].extend({'path': path, 'presence': {'kind': 'absent'}} for path in
                          ('.agentfoundry/config.json', '.agentfoundry/state/manifest.json'))
snapshot_validator.validate(snapshot)
for control in ('.agentfoundry/config.json', '.agentfoundry/state/manifest.json'):
    bad = copy.deepcopy(snapshot)
    bad['entries'] = [entry for entry in bad['entries'] if entry['path'] != control]
    assert list(snapshot_validator.iter_errors(bad)), control
for field, value in [('state_digest', 'invalid'), ('schema', 'agentfoundry.snapshot/v99')]:
    bad = copy.deepcopy(snapshot)
    bad[field] = value
    assert list(snapshot_validator.iter_errors(bad)), field
for presence in [{'kind': 'symlink'}, {'kind': 'absent', 'digest': 'a' * 64},
                 {'kind': 'file', 'digest': 'a' * 64, 'mode': 4095}]:
    bad = copy.deepcopy(snapshot)
    bad['entries'][0]['presence'] = presence
    assert list(snapshot_validator.iter_errors(bad)), presence
import subprocess
output_validator = Draft202012Validator(json.loads((root / 'schemas/command-result.json').read_text()))
if len(sys.argv) > 1:
    binary = Path(sys.argv[1])
else:
    import tomllib
    version = tomllib.loads((root / 'crates/agentfoundry/Cargo.toml').read_text())['package']['version']
    binary = root / 'artifacts' / version / 'agentfoundry-linux-x64'
for args, expected_exit in [(['--version', '--json'], 0), (['--unsupported-flag', '--json'], 2)]:
    result = subprocess.run([str(binary), *args], capture_output=True, text=True)
    assert result.returncode == expected_exit
    output_validator.validate(json.loads(result.stdout))
print('schema validation: contracts and config/byte-image negative boundaries passed')
