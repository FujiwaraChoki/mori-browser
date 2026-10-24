"""Verify an engine upgrade can reuse Git objects without touching local work."""
import subprocess
import sys
import tempfile
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_checkout import seed_repository


class CheckoutSeedTest(unittest.TestCase):
    def test_local_changes_and_cache_history_remain_untouched(self):
        # Kept in OS temp for diagnostics; never recursively delete user paths.
        directory = Path(tempfile.mkdtemp(prefix='mori-checkout-seed-'))
        cache, target = directory / 'old cache', directory / 'new checkout'
        subprocess.run(['git', 'init', '--quiet', str(cache)], check=True)
        (cache / 'source.txt').write_text('committed source\n')
        subprocess.run(['git', '-C', str(cache), 'add', 'source.txt'], check=True)
        subprocess.run(['git', '-C', str(cache), '-c', 'user.name=Mori Test',
                        '-c', 'user.email=test@example.invalid', 'commit', '-qm',
                        'Test cache'], check=True)
        revision = subprocess.check_output(
            ['git', '-C', str(cache), 'rev-parse', 'HEAD'], text=True).strip()
        (cache / 'source.txt').write_text('local work must survive\n')
        (cache / 'untracked.txt').write_text('untracked work\n')

        seed_repository(cache, target, 'https://example.invalid/chromium.git')
        subprocess.run(['git', '-C', str(target), 'checkout', '--force',
                        '--detach', revision], check=True, capture_output=True)
        self.assertEqual((cache / 'source.txt').read_text(), 'local work must survive\n')
        self.assertEqual((cache / 'untracked.txt').read_text(), 'untracked work\n')
        self.assertEqual((target / 'source.txt').read_text(), 'committed source\n')
        self.assertFalse((target / 'untracked.txt').exists())
        self.assertNotIn(b'\r', (target / '.git/objects/info/alternates').read_bytes())
        self.assertEqual(subprocess.check_output(
            ['git', '-C', str(cache), 'rev-parse', 'HEAD'], text=True).strip(), revision)
        self.assertNotEqual(subprocess.run(
            ['git', '-C', str(target), 'symbolic-ref', '-q', 'HEAD'],
            capture_output=True).returncode, 0)


if __name__ == '__main__':
    unittest.main()
