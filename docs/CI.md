# Checks and local desktop builds

## Free hosted checks

[Checks](https://github.com/FujiwaraChoki/mori-browser/actions/workflows/checks.yml)
runs on pushes to `main`, pull requests, and manual dispatch. Standard public
GitHub-hosted Ubuntu 24.04 and Windows 2025 runners check:

- Python/JavaScript/shell syntax and the Chromium version manifest;
- checkout-cache, Windows build-lock, and source-junction regressions;
- workflow restrictions and the installed runner guard's policy;
- Playwright test discovery (the browser integration suites run after a local build);
- native C++ Markdown regressions on Linux and workflow lint with actionlint.

This workflow does not download Chromium. It uses read-only repository tokens
and actions pinned to commit SHAs. Fork workflows from all external contributors
require maintainer approval in the repository's Actions settings.

## Build on Sami's PC

Open [Build on Sami's PC](https://github.com/FujiwaraChoki/mori-browser/actions/workflows/desktop-build.yml),
choose **Run workflow**, keep branch **main**, and select:

| Input | Options |
| --- | --- |
| Platform | `windows`, `linux`, or `both` |
| Operation | `verify`, `build`, or `package` |
| Compiler jobs | `4`, `8`, or `12` |

`verify` is the default: it checks the machine and runs small tooling/native
tests without fetching or compiling Chromium. `build` compiles the browser;
Windows then runs the sandboxed PiP and media integration suites. `package`
also writes installers on Windows or a runtime tarball on Linux. A Linux
verification run checks the runner/toolchain, not the full Chromium port.

Only FujiwaraChoki can run this workflow, from `main`. Runs queue behind one
another and the platform matrix runs one job at a time because both runners
share the same physical PC. Closing Codex does not stop the runners. The PC
must be awake and signed in; after a reboot they start at Windows sign-in.

### Installed machine configuration

| Item | Location/name |
| --- | --- |
| Windows runner | `sami-pc-windows`, label `mori-windows`, application at `C:\mori-ci\runner-windows` |
| Windows engine cache | `C:\mori-build-156` |
| Windows machine paths | `C:\mori-ci\machine.json` (Python, Node, depot_tools, checkout, local artifacts) |
| Linux runner | `sami-pc-linux`, label `mori-linux`, inside WSL distribution `Mori-Linux` |
| Linux account/application | ordinary user `mori`, `/home/mori/actions-runner` |
| Linux engine cache | `/home/mori/.local/share/mori-ci/chromium-156.0.8078.47` |
| Automatic startup | Windows scheduled task `MoriGitHubRunners`, running as Sami at sign-in |
| Startup script and logs | `C:\mori-ci\start-runners.ps1`, `*-runner.stdout.log` and `*-runner.stderr.log` |

The Linux environment is dedicated Ubuntu 24.04 under WSL2, with native ext4
storage for the eventual Chromium cache. It is separate from Docker Desktop.
`ci/linux_build.py` pins Chromium and ungoogled-core through
`windows/chromium.json` and depot_tools through `ci/linux.json`. It initializes
and synchronizes a native Linux checkout on the first actual build, keeps a
patch hash ledger, and serializes writes with `flock`. Unlike the older
`linux/*.sh` wrappers it does not require a matching portablelinux packaging
release. The initial fetch/build is large and can take many hours. Free space
is checked on both the WSL filesystem and the Windows backing drive.
Patch hashes normalize LF/CRLF line endings and accept the corresponding
legacy hashes; changes to patch content still require a fresh checkout.

The Windows job reuses the existing engine cache. Under the checkout lock it
can redirect Chromium's Mori source junction to the checked-out CI revision.
It archives the previous junction under `C:\mori-build-156\mori-overlay-links`;
the developer's source files are preserved. To return a manual build to the
developer checkout, run from that repository:

```powershell
python windows/build.py --setup-only --relink-overlay
```

### Personal-runner boundary

Public pull-request code belongs on GitHub-hosted machines. The installed
`ACTIONS_RUNNER_HOOK_JOB_STARTED` guard rejects other events, users, branches,
repositories, workflows, and jobs before their steps, including checkout.
The guard is copied outside the job workspace and invoked using an isolated
Python interpreter. Its installed paths are `C:\mori-ci\runner-guard.*` and
`/opt/mori-ci/runner-guard.*`; changing the repository copy does not replace
the installed guard. Never approve a change that routes pull-request jobs to
the personal runners or weakens this boundary. These runners execute trusted
owner-authorized code with access to their account; the guard is not a sandbox.

### Local build outputs

No workflow uploads Actions artifacts or enables paid runners/cache storage.
Packaged Windows files stay under `C:\mori-ci\artifacts\<run-id>`; Linux files
stay under `/home/mori/mori-artifacts/<run-id>`. Their paths appear in the run
summary. Manage local disk retention separately. Publishing a GitHub release
is a separate operation.

To start idle/offline runners again:

```powershell
Start-ScheduledTask -TaskName MoriGitHubRunners
```

To stop them, first let any active build finish, then stop the two runner
listeners. Disabling `MoriGitHubRunners` in Task Scheduler prevents automatic
startup. Runner credentials stay in their application directories, outside Git.

macOS build runners and iOS CI are deferred.
