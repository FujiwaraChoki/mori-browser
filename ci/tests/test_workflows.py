"""Keep public pull-request code away from persistent personal runners."""
from pathlib import Path
import unittest
import yaml

ROOT = Path(__file__).resolve().parents[2]


class WorkflowBoundaryTest(unittest.TestCase):
    def test_hosted_checks_do_not_use_personal_runners(self):
        checks = yaml.safe_load((ROOT / '.github/workflows/checks.yml').read_text())
        self.assertIn('pull_request', checks['on'])
        self.assertEqual(checks['jobs']['tooling']['strategy']['matrix']['os'],
                         ['ubuntu-24.04', 'windows-2025'])
        self.assertEqual(checks['permissions'], {'contents': 'read'})

    def test_personal_builds_require_owner_main_and_manual_dispatch(self):
        workflow = yaml.safe_load((ROOT / '.github/workflows/desktop-build.yml').read_text())
        self.assertEqual(set(workflow['on']), {'workflow_dispatch'})
        for name in ('plan', 'build'):
            guard = workflow['jobs'][name]['if']
            self.assertIn("github.repository == 'FujiwaraChoki/mori-browser'", guard)
            self.assertIn("github.actor == 'FujiwaraChoki'", guard)
            self.assertIn("github.ref == 'refs/heads/main'", guard)
        build = workflow['jobs']['build']
        self.assertIn('self-hosted', build['runs-on'])
        self.assertEqual(build['strategy']['max-parallel'], 1)
        self.assertFalse(workflow['concurrency']['cancel-in-progress'])
        self.assertEqual(workflow['permissions'], {'contents': 'read'})
        checkout = build['steps'][0]['with']
        self.assertEqual(checkout['ref'], '${{ github.sha }}')
        self.assertFalse(checkout['persist-credentials'])

    def test_actions_are_pinned_and_outputs_stay_local(self):
        import re
        for path in (ROOT / '.github/workflows').glob('*.yml'):
            workflow = yaml.safe_load(path.read_text())
            for job in workflow['jobs'].values():
                for step in job.get('steps', []):
                    if 'uses' in step:
                        self.assertRegex(step['uses'], r'@[a-f0-9]{40}$')
                        self.assertNotIn('upload-artifact', step['uses'])
