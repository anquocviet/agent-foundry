#!/usr/bin/env python3
"""T-042/T-046: public update rejection through the real HTTP/download/probe path.

Usage: python3 -B tests/acceptance/release_negative.py FAULT_BINARY
Local synthetic release only; never contacts or changes production releases.
"""
import copy
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import time

from native_matrix import snapshot

PLATFORMS = ('aarch64-apple-darwin', 'x86_64-apple-darwin',
             'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu',
             'x86_64-pc-windows-msvc')


def cli(binary, root, env, *args, code=0, error='OK'):
    if code == 1:
        code = 7 if error in ('NETWORK_ERROR', 'RELEASE_UNAVAILABLE') else 5
    result = subprocess.run([str(binary), *args, '--directory', str(root), '--json'],
                            env=env, capture_output=True, text=True, timeout=100)
    value = json.loads(result.stdout)  # Also rejects multiple envelopes/trailing output.
    assert result.returncode == code, (args, result.returncode, value, result.stderr)
    assert value['command'] == args[0], value
    assert value['schema'] == 'agentfoundry.command-result/v1', value
    assert value['status'] == ('ok' if code == 0 else 'blocked'), value
    assert value['code'] == error, value
    assert value['mutation'] == ('applied' if args[0] == 'install' else 'none'), value
    assert isinstance(value['data'], dict), value
    assert all(isinstance(item, str) for item in value['warnings']), value
    assert all(set(item) == {'message', 'next_action'} and
               all(isinstance(text, str) for text in item.values())
               for item in value['errors']), value
    assert bool(value['errors']) == (code != 0), value
    assert set(value) == {'schema', 'command', 'status', 'code', 'mutation',
                          'data', 'warnings', 'errors'}, value
    return value


def main():
    # ponytail: Unix executable fixture; native Windows needs a compiled probe executable.
    assert os.name == 'posix', 'this probe fixture requires Unix executable scripts'
    binary = Path(sys.argv[1]).resolve()
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(('AF_', 'AGENTFOUNDRY_'))}
    with tempfile.TemporaryDirectory(prefix='af-release-negative-') as temp:
        work = Path(temp)
        root = work / 'consumer'
        root.mkdir()
        (root / 'private.txt').write_text('preserve consumer bytes\n')
        identity = cli(binary, root, env, 'version')['data']
        assert identity['fault_injection'] is True, identity
        exported = cli(binary, root, env, 'distribution')['data']
        cli(binary, root, env, 'install')
        selected = root / 'scripts/bin/agentfoundry'
        platform = ('aarch64' if os.uname().machine in ('aarch64', 'arm64') else 'x86_64')
        platform += '-apple-darwin' if sys.platform == 'darwin' else '-unknown-linux-gnu'
        version = str(int(identity['version'].split('.')[0]) + 1) + '.0.0'
        tag = 'v' + version
        prefix = '/anquocviet/agent-foundry/releases/download/' + tag + '/'
        latest = '/repos/anquocviet/agent-foundry/releases/latest'
        trace = work / 'launches.jsonl'
        export_path = work / 'export.json'
        probe_identity = copy.deepcopy(exported)
        probe_identity.update(version=version, source='a' * 40)
        export_path.write_text(json.dumps(probe_identity))
        # Log every execution independently of the CLI result. Any handoff or activation
        # would add a non-distribution launch or change the complete consumer snapshot.
        probe = ('#!' + sys.executable + '\n'
                 'import hashlib,json,pathlib,sys\n'
                 f'with open({str(trace)!r}, "a") as log: log.write(json.dumps(sys.argv[1:])+"\\n")\n'
                 'if sys.argv[1:] != ["distribution", "--json"]: sys.exit(97)\n'
                 f'data=json.loads(pathlib.Path({str(export_path)!r}).read_text())\n'
                 'data["binary_digest"]=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()\n'
                 'print(json.dumps(data))\n'
                 '# hash fixture: A\n').encode()
        sha = hashlib.sha256(probe).hexdigest()
        metadata = dict(schema='agentfoundry.release/v1', version=version, tag=tag,
                        source_commit='a' * 40, payload_digest=exported['payload_digest'],
                        state_schema_min=1, state_schema_max=1, handoff_protocol=1,
                        assets=[dict(platform=p, name='agentfoundry-' + p,
                                     size=len(probe), sha256=sha,
                                     checksum_name='agentfoundry-' + p + '.sha256')
                                for p in PLATFORMS])
        asset = next(a for a in metadata['assets'] if a['platform'] == platform)
        api = dict(tag_name=tag, draft=False, prerelease=False,
                   assets=[dict(name=name) for name in
                           ['agentfoundry-release.json'] +
                           [a[key] for a in metadata['assets']
                            for key in ('name', 'checksum_name')]])
        routes = {}
        requests = []

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                requests.append(self.path)
                status, body, headers, delay, truncated = routes.get(
                    self.path, (404, b'', {}, 0, False))
                if status == 0:
                    self.connection.shutdown(socket.SHUT_RDWR)
                    self.connection.close()
                    return
                self.send_response(status)
                self.send_header('Content-Length', str(len(body) + (100 if truncated else 0)))
                for key, value in headers.items():
                    self.send_header(key, value)
                self.end_headers()
                try:
                    if delay:
                        self.wfile.write(body[:1])
                        self.wfile.flush()
                        time.sleep(delay)
                        self.wfile.write(body[1:])
                    else:
                        self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass
                self.close_connection = True

        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        server.daemon_threads = True
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        env['AGENTFOUNDRY_RELEASE_HTTP_PORT'] = str(server.server_port)
        count = 0

        def reset(meta=None, pointer=None, candidate=probe):
            routes.clear()
            routes[latest] = (200, json.dumps(pointer or api).encode(), {}, 0, False)
            routes[prefix + 'agentfoundry-release.json'] = (
                200, json.dumps(meta or metadata).encode(), {}, 0, False)
            routes[prefix + asset['name']] = (200, candidate, {}, 0, False)
            routes[prefix + asset['checksum_name']] = (
                200, f"{sha}  {asset['name']}\n".encode(), {}, 0, False)
            requests.clear()
            trace.unlink(missing_ok=True)

        def check(name, error, launches=0, code=1, message=None, args=('update',)):
            nonlocal count
            before = snapshot(root)
            started = time.monotonic()
            value = cli(selected, root, env, *args, code=code, error=error)
            elapsed = time.monotonic() - started
            assert snapshot(root) == before, name
            actual = [json.loads(line) for line in trace.read_text().splitlines()] if trace.exists() else []
            assert actual == [['distribution', '--json']] * launches, (name, actual)
            if message:
                assert any(message in e['message'] for e in value['errors']), (name, value)
            count += 1
            print(f'{name}: {error}; probes={launches}; consumer unchanged; {elapsed:.2f}s', flush=True)
            return elapsed

        try:
            reset()
            check('valid release observation', 'OK', code=0, args=('status', '--check-updates'))
            reset()
            check('valid verified probe preview', 'OK', launches=1, code=0, args=('update', '--dry-run'))
            for field, wrong, message in (
                    ('version', '99.0.0', 'candidate version mismatch'),
                    ('source', 'b' * 40, 'candidate source identity mismatch'),
                    ('payload_digest', 'c' * 64, 'candidate payload_digest mismatch')):
                changed = dict(probe_identity, **{field: wrong})
                export_path.write_text(json.dumps(changed))
                reset()
                check('probe ' + field, 'INTEGRITY_FAILURE', launches=1, message=message)
            export_path.write_text(json.dumps(probe_identity))
            wrong_hash_probe = probe[:-2] + b'B\n'
            assert len(wrong_hash_probe) == len(probe)
            wrong_hash_path = work / 'wrong-hash-probe'
            wrong_hash_path.write_bytes(wrong_hash_probe)
            wrong_hash_path.chmod(0o755)
            trace.unlink(missing_ok=True)
            proof = subprocess.run([str(wrong_hash_path), 'distribution', '--json'],
                                   env=env, capture_output=True, timeout=30)
            assert proof.returncode == 0, proof.stderr
            assert trace.read_text().splitlines() == ['["distribution", "--json"]']
            reset(candidate=wrong_hash_probe)
            check('asset hash mismatch', 'INTEGRITY_FAILURE', message='digest mismatch')
            reset(candidate=probe[:-1])
            check('asset size mismatch', 'INTEGRITY_FAILURE', message='size mismatch')
            for label, body in (
                    ('sidecar digest', f"{'0' * 64}  {asset['name']}\n".encode()),
                    ('sidecar filename', f'{sha}  wrong-name\n'.encode()),
                    ('sidecar invalid UTF-8', b'\xff')):
                reset()
                routes[prefix + asset['checksum_name']] = (200, body, {}, 0, False)
                check(label, 'INTEGRITY_FAILURE')
            for field, wrong in (('source_commit', 'invalid'), ('payload_digest', 'invalid'),
                                 ('handoff_protocol', 2), ('tag', 'v99.0.0')):
                reset(meta=dict(metadata, **{field: wrong}))
                check('invalid metadata ' + field, 'INTEGRITY_FAILURE')
                assert prefix + asset['name'] not in requests
            pointer = copy.deepcopy(api)
            pointer['assets'][0] = pointer['assets'][1]
            reset(pointer=pointer)
            check('duplicate API asset', 'INTEGRITY_FAILURE')
            pointer = dict(api, tag_name='v99.0.0')
            reset(pointer=pointer)
            routes['/anquocviet/agent-foundry/releases/download/v99.0.0/agentfoundry-release.json'] = routes[prefix + 'agentfoundry-release.json']
            check('pointer metadata tag mismatch', 'INTEGRITY_FAILURE')
            for path, error in ((latest, 'NETWORK_ERROR'),
                                (prefix + 'agentfoundry-release.json', 'INVALID_STATE')):
                reset()
                routes[path] = (200, b'{', {}, 0, False)
                check('malformed JSON ' + path, error)
            for label, response, error in (
                    ('HTTP 404', (404, b'', {}, 0, False), 'RELEASE_UNAVAILABLE'),
                    ('HTTP 503', (503, b'', {}, 0, False), 'NETWORK_ERROR'),
                    ('connection closed', (0, b'', {}, 0, False), 'NETWORK_ERROR'),
                    ('truncated response', (200, b'{', {}, 0, True), 'NETWORK_ERROR'),
                    ('untrusted redirect', (302, b'', {'Location': 'https://example.com/evil'}, 0, False), 'NETWORK_ERROR'),
                    ('redirect loop', (302, b'', {'Location': latest}, 0, False), 'NETWORK_ERROR')):
                reset()
                routes[latest] = response
                check(label, error)
                if label == 'untrusted redirect':
                    assert requests == [latest], requests
            reset()
            routes[latest] = (200, json.dumps(api).encode(), {}, 32, False)
            elapsed = check('slow body deadline', 'NETWORK_ERROR')
            assert 25 <= elapsed < 40, elapsed
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
        print(f'release negative acceptance passed: {count} scenarios; local synthetic transport only')


if __name__ == '__main__':
    main()
