import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('runner_guard', Path(__file__).resolve().parents[1] / 'runner_guard.py')
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class RunnerGuardTest(unittest.TestCase):
    def test_owner_main_dispatch_is_accepted(self):
        self.assertEqual(guard.denied_fields(dict(guard.EXPECTED)), [])

    def test_each_missing_or_changed_identity_is_rejected(self):
        for field in guard.EXPECTED:
            with self.subTest(field=field):
                env = dict(guard.EXPECTED)
                env.pop(field)
                self.assertIn(field, guard.denied_fields(env))
                env[field] = 'untrusted'
                self.assertIn(field, guard.denied_fields(env))

    def test_pull_request_and_other_workflow_are_rejected(self):
        env = dict(guard.EXPECTED)
        env.update(GITHUB_EVENT_NAME='pull_request', GITHUB_REF='refs/pull/7/merge',
                   GITHUB_WORKFLOW_REF='FujiwaraChoki/mori-browser/.github/workflows/checks.yml@refs/pull/7/merge')
        self.assertEqual(len(guard.denied_fields(env)), 3)
