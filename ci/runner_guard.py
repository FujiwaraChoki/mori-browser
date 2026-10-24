"""Installed outside job workspaces; reject personal-runner jobs before steps.

Run this with Python's -I -S switches from the runner's JOB_STARTED hook.
This is an additional boundary, not a sandbox for code the owner authorizes.
"""
import os
import sys

EXPECTED = {
    'GITHUB_REPOSITORY': 'FujiwaraChoki/mori-browser',
    'GITHUB_ACTOR': 'FujiwaraChoki',
    'GITHUB_TRIGGERING_ACTOR': 'FujiwaraChoki',
    'GITHUB_EVENT_NAME': 'workflow_dispatch',
    'GITHUB_REF': 'refs/heads/main',
    'GITHUB_WORKFLOW_REF': 'FujiwaraChoki/mori-browser/.github/workflows/desktop-build.yml@refs/heads/main',
    'GITHUB_JOB': 'build',
}


def denied_fields(environment):
    return [key for key, expected in EXPECTED.items() if environment.get(key) != expected]


def main():
    denied = denied_fields(os.environ)
    if denied:
        print('Personal runner rejected this job before checkout: ' + ', '.join(denied), file=sys.stderr)
        return 1
    print('Personal runner accepted the owner-dispatched main desktop workflow.')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
