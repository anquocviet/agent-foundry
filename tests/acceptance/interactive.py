#!/usr/bin/env python3
"""Native POSIX PTY proof of core interactive override dry-run, not PowerShell proof."""
import errno
import os
from pathlib import Path
import pty
import select
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='agentfoundry-interactive-') as directory:
    root = Path(directory) / 'consumer'
    root.mkdir()
    agents = root / 'AGENTS.md'
    agents.write_bytes(b'private instructions\r\n')
    before = {str(p.relative_to(root)): p.read_bytes() for p in root.rglob('*') if p.is_file()}
    master, slave = pty.openpty()
    process = subprocess.Popen([str(binary), 'install', '--directory', str(root), '--dry-run'], stdin=slave, stdout=slave, stderr=slave)
    os.close(slave)
    output = bytearray()
    answered = False
    while True:
        ready, _, _ = select.select([master], [], [], 20)
        if not ready:
            process.kill()
            raise AssertionError('interactive planner timed out')
        try:
            chunk = os.read(master, 65536)
        except OSError as error:
            if error.errno == errno.EIO:
                break
            raise
        if not chunk:
            break
        output.extend(chunk)
        if not answered and b'[s]top:' in output:
            os.write(master, b'o\n')
            answered = True
    os.close(master)
    assert process.wait(timeout=10) == 0, output.decode(errors='replace')
    assert answered, output.decode(errors='replace')
    assert before == {str(p.relative_to(root)): p.read_bytes() for p in root.rglob('*') if p.is_file()}
    assert not (root / '.agentfoundry').exists()
print('POSIX interactive override dry-run preserved target without runtime artifacts')
