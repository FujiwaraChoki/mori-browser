const {defineConfig} = require('playwright/test');
const path = require('node:path');
const {default_checkout} = require('../chromium.json');
module.exports = defineConfig({
  testDir: __dirname,
  testMatch: 'pip.spec.cjs',
  workers: 1,
  timeout: 30000,
  outputDir: '../../dist/pip-test-results',
  use: {
    headless: true,
    viewport: {width: 1000, height: 800},
    launchOptions: {
      executablePath: process.env.MORI_TEST_BROWSER || path.join(default_checkout, 'src/out/Default/chrome.exe'),
      chromiumSandbox: true,
    },
  },
});
