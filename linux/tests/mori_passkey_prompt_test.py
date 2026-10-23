#!/usr/bin/env python3
"""Exercise the production prompt probe through Playwright CLI, with fake accounts.

Example: python3 linux/tests/mori_passkey_prompt_test.py --cli 'playwright-cli'
Use --config to select an isolated Chromium/Mori executable. No real vault used.
"""
import argparse
import json
from pathlib import Path
import shlex
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("--cli", default="playwright-cli")
parser.add_argument("--config")
options = parser.parse_args()
cli = shlex.split(options.cli) + ["-s=mori-passkey-tests"]
header = (Path(__file__).parent / "../mori/mori_passkeys.h").read_text()
probe = header.split('kFocusPrompt[] = R"JS(', 1)[1].split(')JS";', 1)[0]
probe = probe.replace("ACCOUNT_JSON", json.dumps("alice@example.com"))

code = r"""async page => {
  const probe = PROBE_JSON;
  const prompt = (account = 'alice@example.com', label = 'Use passkey') =>
    `<div role="button" tabindex="0" aria-label="Sign in with GitHub">
       <span>GitHub</span><span>${account}</span><span>${label}</span>
     </div>`;
  let count = 0;
  const check = async (html, expected) => {
    await page.setContent(html);
    await page.evaluate(() => delete window.__moriPasskeyConfirmationRequested);
    const result = await page.evaluate(probe);
    if (result !== expected) throw new Error(`Expected ${expected}; got ${result}`);
    ++count;
  };
  await check(prompt(), 'ready');
  if (await page.evaluate(probe) !== 'confirmation_already_requested')
    throw new Error('Repeated confirmation was not blocked');
  ++count;
  await page.evaluate(() => {
    window.confirmations = 0;
    window.trusted = false;
    document.activeElement.addEventListener('keydown', e => {
      if (e.key === 'Enter') { ++window.confirmations; window.trusted = e.isTrusted; }
    });
  });
  // Merely probing/focusing must never activate the credential.
  if (await page.evaluate(() => window.confirmations) !== 0) throw new Error('Probe activated');
  await page.keyboard.press('Enter');
  if (!(await page.evaluate(() => window.confirmations === 1 && window.trusted)))
    throw new Error('Native Enter did not activate exactly once');
  ++count;
  await check(prompt('bob@example.com'), 'no_unique_matching_passkey');
  await check(prompt('alice@example.com.evil'), 'no_unique_matching_passkey');
  await check(prompt('alice@example.com', 'Sign in'), 'no_unique_matching_passkey');
  await check(prompt() + prompt(), 'no_unique_matching_passkey');
  await check(`<div hidden>${prompt()}</div>`, 'no_unique_matching_passkey');
  await check(`<div style="opacity:0">${prompt()}</div>`, 'no_unique_matching_passkey');
  await check(`<div inert>${prompt()}</div>`, 'no_unique_matching_passkey');
  await check(prompt().replace('tabindex="0"', 'tabindex="0" aria-disabled="true"'),
    'no_unique_matching_passkey');
  await check('<button><span>Unlock 1Password to sign in</span>Unlock</button>',
    'needs_user_authentication');
  await check(prompt('alice@example.com', 'Passkey verwenden'), 'no_unique_matching_passkey');
  await check('<p>Loading</p>', 'no_unique_matching_passkey');
  const defaultProbe = probe.replace('const account = "alice@example.com";',
    'const account = "@default";');
  await page.setContent(prompt('unknown-label@example.com'));
  await page.evaluate(() => delete window.__moriPasskeyConfirmationRequested);
  if (await page.evaluate(defaultProbe) !== 'ready') throw new Error('Default account failed');
  ++count;
  await page.setContent(prompt('one@example.com') + prompt('two@example.com'));
  await page.evaluate(() => delete window.__moriPasskeyConfirmationRequested);
  if (await page.evaluate(defaultProbe) !== 'no_unique_matching_passkey')
    throw new Error('Ambiguous default account accepted');
  ++count;
  return `PASS: ${count} passkey prompt checks`;
}""".replace("PROBE_JSON", json.dumps(probe))


def run(*args):
    result = subprocess.run(cli + list(args), text=True, capture_output=True, check=True)
    return result.stdout + result.stderr


with tempfile.TemporaryDirectory(prefix="mori-passkey-tests-") as directory:
    source = Path(directory) / "test.js"
    source.write_text(code)
    try:
        args = ["open", "about:blank"]
        if options.config:
            args.append("--config=" + str(Path(options.config).resolve()))
        run(*args)
        output = run("run-code", "--filename=" + str(source))
        if "PASS: 16 passkey prompt checks" not in output:
            raise RuntimeError(output)
        print("PASS: 16 passkey prompt checks (including trusted native Enter)")
    finally:
        run("close")
