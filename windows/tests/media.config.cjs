const {defineConfig} = require('playwright/test');
const base = require('./pip.config.cjs');

module.exports = defineConfig(base, {
  testMatch: 'media.spec.cjs',
  outputDir: '../../dist/media-test-results',
});
