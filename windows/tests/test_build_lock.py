"""Exercise the checkout lock with separate Windows build processes."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest


HELPER = '''
from pathlib import Path
import sys, time
sys.path.insert(0, sys.argv[1])
from build_config import checkout_build_lock
checkout = Path(sys.argv[2])
with checkout_build_lock(checkout, poll_interval=0.05):
    (checkout / (sys.argv[3] + '.ready')).write_text('acquired')
    if sys.argv[3] == 'first':
        deadline = time.monotonic() + 20
        while not (checkout / 'release').exists():
            if time.monotonic() > deadline:
                raise RuntimeError('Test did not release the first build')
            time.sleep(0.05)
'''


@unittest.skipUnless(os.name == 'nt', 'Windows build lock')
class BuildLockTest(unittest.TestCase):
    def test_builds_wait_for_the_checkout_owner(self):
        checkout = Path(tempfile.mkdtemp(prefix='mori-build-lock-'))
        windows = Path(__file__).resolve().parents[1]
        children = []

        def start(name):
            child = subprocess.Popen(
                [sys.executable, '-c', HELPER, str(windows), str(checkout), name],
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                creationflags=subprocess.CREATE_NO_WINDOW)
            children.append(child)
            return child

        try:
            first = start('first')
            deadline = time.monotonic() + 10
            while not (checkout / 'first.ready').exists():
                if first.poll() is not None or time.monotonic() > deadline:
                    self.fail('First build did not acquire the checkout')
                time.sleep(0.05)
            second = start('second')
            time.sleep(0.4)
            self.assertIsNone(second.poll())
            self.assertFalse((checkout / 'second.ready').exists())
            (checkout / 'release').write_text('continue')
            self.assertEqual(first.wait(timeout=10), 0, first.stdout.read())
            self.assertEqual(second.wait(timeout=10), 0, second.stdout.read())
            self.assertTrue((checkout / 'second.ready').exists())
        finally:
            (checkout / 'release').write_text('continue')
            for child in children:
                if child.poll() is None:
                    child.kill()
                    child.wait(timeout=10)
                child.stdout.close()
