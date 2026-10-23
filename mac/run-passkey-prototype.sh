#!/bin/bash
# Launch the staged prototype with standing authorization for the US vault
# sign-in origins. Uses Mori's existing profile and installed extension.
set -euo pipefail
prototype_repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
prototype_app="${prototype_repo}/dist/passkey-prototype/Mori.app"
if [[ ! -d "${prototype_app}" ]]; then
    echo "Build Chromium, then run: mac/package.sh dist/passkey-prototype/Mori.app" >&2
    exit 1
fi
if pgrep -f '/Mori\.app/Contents/MacOS/Chromium([[:space:]]|$)' >/dev/null; then
    echo "Quit Mori first so the new executable and passkey startup setting take effect." >&2
    exit 1
fi
prototype_accounts='{"https://my.1password.com":"@default","https://start.1password.com":"@default"}'
/usr/bin/open -n "${prototype_app}" --args \
    --restore-last-session \
    "--mori-auto-passkey-accounts=${prototype_accounts}" \
    https://my.1password.com
echo "Prototype opened. Start a new Mori assistant chat and ask it to sign in with your 1Password passkey."
