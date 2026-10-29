"""Stdlib-only Git reader: no shell, index refresh, temp files or bytecode writes."""
import os
import subprocess
import threading
import time


def git_read(repository, *arguments, limit=8 * 1024 * 1024):
    environment = os.environ.copy()
    environment['GIT_OPTIONAL_LOCKS'] = '0'
    environment['GIT_TERMINAL_PROMPT'] = '0'
    command = ['git', '--no-optional-locks', '-c', 'maintenance.auto=false',
               '-c', 'gc.auto=0', '-c', 'core.fsmonitor=false', '-C', str(repository), *arguments]
    process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, env=environment)
    output = [bytearray(), bytearray()]
    overflow = threading.Event()
    failures = []
    def drain(stream, index):
        try:
            while True:
                chunk = stream.read(min(65536, limit + 1 - len(output[index])))
                if not chunk:
                    break
                output[index].extend(chunk)
                if len(output[index]) > limit:
                    overflow.set()
                    break
        except OSError as error:
            failures.append(error)
        finally:
            stream.close()
    workers = [threading.Thread(target=drain, args=(stream, index), daemon=True)
               for index, stream in enumerate((process.stdout, process.stderr))]
    for worker in workers:
        worker.start()
    deadline = time.monotonic() + 30
    try:
        while process.poll() is None:
            if overflow.is_set() or time.monotonic() >= deadline:
                process.kill()
                raise OSError('INPUT_LIMIT_EXCEEDED: Git output/time limit; incomplete evidence rejected')
            time.sleep(0.01)
        for worker in workers:
            worker.join(timeout=max(0, deadline - time.monotonic()))
        if any(worker.is_alive() for worker in workers) or overflow.is_set():
            raise OSError('INPUT_LIMIT_EXCEEDED: Git pipes exceeded evidence limit')
        if failures:
            raise failures[0]
        return subprocess.CompletedProcess(command, process.returncode, bytes(output[0]), bytes(output[1]))
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
