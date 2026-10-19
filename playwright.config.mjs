// All fixture servers are isolated from the developer’s wallet access configuration.
process.env.CWBBOT_ADMIN = "";
import { defineConfig, devices } from '@playwright/test';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';
// lifecycle.spec drives `make start2`, which rebuilds dist/ and the binaries that
// every other fixture server is serving; run it on its own with RUN_LIFECYCLE=1.
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'causewaybay-e2e-'));
// PW_PORT and PW_OUTPUT let several runs share one checkout (each run gets its own
// fixture server and results folder); the defaults are the usual single run.
const port = process.env.PW_PORT || '3099';
const output = process.env.PW_OUTPUT || 'test-results';
export default defineConfig({
  testDir: './tests/e2e',
  testIgnore: [...(process.env.RUN_LIVE_AI === '1' ? [] : ['**/live.spec.mjs', '**/automatic-capture.spec.mjs']), ...(process.env.TEST_PACKAGE === '1' ? [] : ['**/package.spec.mjs']), ...(process.env.RUN_NATIVE_CAPTURE === '1' ? [] : ['**/native-capture.spec.mjs']), ...(process.env.RUN_NATIVE_NOTES === '1' ? [] : ['**/native-notes.spec.mjs']), ...(process.env.RUN_NATIVE_SCREEN === '1' ? [] : ['**/native-screen.spec.mjs']), ...(process.env.RUN_LIVE_WALLET === '1' ? [] : ['**/wallet-live.spec.mjs']), ...(process.env.RUN_LIFECYCLE === '1' ? [] : ['**/lifecycle.spec.mjs'])],
  fullyParallel: false,
  workers: 1,
  timeout: 45000,
  expect: { timeout: 15000 },
  outputDir: output,
  reporter: [['list'], ['html', { open: 'never', outputFolder: process.env.PW_OUTPUT ? path.join(output, 'html-report') : 'playwright-report' }]],
  use: { baseURL: `http://127.0.0.1:${port}`, reducedMotion: 'reduce', trace: 'retain-on-failure', screenshot: 'only-on-failure' },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    { name: 'webkit-ipad', testMatch: ['**/audio.spec.mjs', '**/everyday.spec.mjs', '**/session-history.spec.mjs', '**/textedit-enrollment.spec.mjs', '**/native-screen.spec.mjs', '**/menu-ui.spec.mjs', '**/cloud.spec.mjs', '**/ai-policy.spec.mjs', '**/motion.spec.mjs', '**/wiki.spec.mjs', '**/invisible-ai.spec.mjs', '**/local-ai.spec.mjs', '**/ondevice-models.spec.mjs', '**/erased-epoch.spec.mjs', '**/device-sync.spec.mjs', '**/capture-image.spec.mjs', '**/fresh-context.spec.mjs', '**/capture-privacy.spec.mjs', '**/file-context.spec.mjs', '**/responsive.spec.mjs', '**/theme.spec.mjs', '**/connection.spec.mjs', '**/ux-flow.spec.mjs', '**/video-upload.spec.mjs', '**/coding.spec.mjs'], use: { ...devices['iPad Mini'] } },
    { name: 'webkit-iphone', testMatch: ['**/audio.spec.mjs', '**/everyday.spec.mjs', '**/session-history.spec.mjs', '**/textedit-enrollment.spec.mjs', '**/native-screen.spec.mjs', '**/menu-ui.spec.mjs', '**/cloud.spec.mjs', '**/ai-policy.spec.mjs', '**/motion.spec.mjs', '**/wiki.spec.mjs', '**/invisible-ai.spec.mjs', '**/local-ai.spec.mjs', '**/ondevice-models.spec.mjs', '**/erased-epoch.spec.mjs', '**/device-sync.spec.mjs', '**/capture-image.spec.mjs', '**/fresh-context.spec.mjs', '**/capture-privacy.spec.mjs', '**/file-context.spec.mjs', '**/responsive.spec.mjs', '**/theme.spec.mjs', '**/connection.spec.mjs', '**/ux-flow.spec.mjs', '**/video-upload.spec.mjs', '**/coding.spec.mjs'], use: { ...devices['iPhone 13'] } },
  ],
  webServer: [{
    command: './target/debug/causewaybay-server',
    url: `http://127.0.0.1:${port}/api/health`,
    timeout: 120000,
    reuseExistingServer: false,
    env: { BIND_ADDR: `127.0.0.1:${port}`, DATA_DIR: dataDir, DEMO_MODE: '1', SEMANTIC_SEARCH: 'off', APP_TOKEN: '', CAUSEWAYBAY_CONFIG_DIR: dataDir, ANTHROPIC_API_KEY: '', OPENAI_API_KEY: '', GEMINI_API_KEY: '', XAI_API_KEY: '' }
  }, ...(process.env.RUN_LIVE_AI === '1' ? [{ command: './target/debug/causewaybay-server', url: 'http://127.0.0.1:3100/api/health', timeout: 120000, reuseExistingServer: false, env: { BIND_ADDR: '127.0.0.1:3100', DATA_DIR: path.join(dataDir, 'live'), CAUSEWAYBAY_CONFIG_DIR: path.join(dataDir, 'live-config'), DEMO_MODE: '0', SEMANTIC_SEARCH: 'off', APP_TOKEN: '', ANTHROPIC_API_KEY: '', OPENAI_API_KEY: '', GEMINI_API_KEY: '' } }] : [])]
});
