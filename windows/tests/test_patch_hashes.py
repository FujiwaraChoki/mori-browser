"""Text checkout normalization must not invalidate a verified engine cache."""
import hashlib
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from build_config import patch_content_hashes


class PatchHashTest(unittest.TestCase):
    def test_lf_and_crlf_accept_both_legacy_hashes(self):
        lf = b'--- a/file\n+++ b/file\n@@ -1 +1 @@\n-before\n+after\n'
        crlf = lf.replace(b'\n', b'\r\n')
        canonical, compatible = patch_content_hashes(lf)
        self.assertEqual(patch_content_hashes(crlf), (canonical, compatible))
        self.assertIn(hashlib.sha256(lf).hexdigest(), compatible)
        self.assertIn(hashlib.sha256(crlf).hexdigest(), compatible)

    def test_content_and_whitespace_changes_are_rejected(self):
        before = b'+return 1;\n'
        canonical, compatible = patch_content_hashes(before)
        for changed in (b'+return 2;\n', b'+return  1;\n', b'+return 1;'):
            self.assertNotIn(hashlib.sha256(changed).hexdigest(), compatible)
            self.assertNotEqual(patch_content_hashes(changed)[0], canonical)
