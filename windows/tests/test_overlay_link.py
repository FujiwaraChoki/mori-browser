"""A CI checkout may replace the source junction, never the developer's files."""
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from build_config import link_mori_sources


@unittest.skipUnless(os.name == 'nt', 'Windows junctions')
class OverlayLinkTest(unittest.TestCase):
    def test_relink_preserves_previous_source_and_requires_opt_in(self):
        root = Path(tempfile.mkdtemp(prefix='mori-ci-junction-'))
        source = root / 'engine/src'
        first, second = root / 'developer', root / 'ci'
        for repo in (first, second):
            (repo / 'linux/mori').mkdir(parents=True)
            for name in ('mori_sidebar.cc', 'mori_toolbar_layout.cc'):
                (repo / 'linux/mori' / name).write_text('local source must survive')
        link_mori_sources(first, source)
        overlay = source / 'chrome/browser/ui/views/mori'
        with self.assertRaises(RuntimeError):
            link_mori_sources(second, source)
        link_mori_sources(second, source, allow_relink=True)
        self.assertEqual(overlay.resolve(), (second / 'linux/mori').resolve())
        archived = list((source.parent / 'mori-overlay-links').iterdir())
        self.assertEqual(len(archived), 1)
        self.assertTrue(archived[0].is_junction())
        self.assertEqual(archived[0].resolve(), (first / 'linux/mori').resolve())
        self.assertEqual((first / 'linux/mori/mori_sidebar.cc').read_text(),
                         'local source must survive')

    def test_real_directory_is_never_replaced(self):
        root = Path(tempfile.mkdtemp(prefix='mori-ci-real-source-'))
        source = root / 'engine/src'
        overlay = source / 'chrome/browser/ui/views/mori'
        overlay.mkdir(parents=True)
        marker = overlay / 'uncommitted.txt'
        marker.write_text('keep')
        with self.assertRaises(RuntimeError):
            link_mori_sources(root / 'ci', source, allow_relink=True)
        self.assertEqual(marker.read_text(), 'keep')
