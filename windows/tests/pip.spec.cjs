// Run using Playwright's CLI with the freshly built Mori executable.
// All browsers use headless mode and isolated temporary profiles.
const {test, expect, chromium} = require('playwright/test');
const http = require('node:http');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const {default_checkout, version: chromiumVersion} = require('../chromium.json');
const browserExecutable = process.env.MORI_TEST_BROWSER || path.join(default_checkout, 'src/out/Default/chrome.exe');
let server, url, sample;

const videoScript = `
  window.addVideo = async () => {
    const canvas = document.createElement('canvas');
    canvas.width = 640; canvas.height = 360;
    const ctx = canvas.getContext('2d');
    const stream = canvas.captureStream(10);
    const video = document.createElement('video');
    video.width = 640; video.height = 360; video.muted = true;
    video.srcObject = stream;
    document.querySelector('#player').prepend(video);
    setInterval(() => { ctx.fillStyle = 'green'; ctx.fillRect(0, 0, 640, 360); }, 100);
    await video.play();
  };
  if (!location.search.includes('dynamic')) addVideo();
`;
test.beforeAll(async () => {
  sample = await fs.readFile(process.env.MORI_TEST_MEDIA ||
    path.join(default_checkout, 'src/content/test/data/media/bear-320x240.webm'));
  server = http.createServer((req, res) => {
    if (req.url === '/sample.webm') {
      res.setHeader('Content-Type', 'video/webm');
      res.end(sample);
      return;
    }
    res.setHeader('Content-Type', 'text/html');
    if (req.url === '/native') {
      res.end(`<video width="640" height="360" loop src="/sample.webm"></video>
        <button id="play" onclick="document.querySelector('video').play()">Play</button>
        <a id="other" target="_blank" href="/video?dynamic">Other tab</a>`);
    } else if (req.url.startsWith('/embed')) {
      // A distinct origin (127.0.0.1 rather than localhost) tests frame injection.
      res.end(`<iframe style="width:750px;height:480px" allow="picture-in-picture" src="${url.replace('localhost', '127.0.0.1')}/video"></iframe>`);
    } else {
      res.end(`<style>body{margin:0}#player{position:relative;width:640px;height:360px}
        #controls{position:absolute;inset:0}</style><div id="player"><div id="controls"></div></div>
        <button id="outside" style="margin-top:80px">Outside video</button><script>${videoScript}</script>`);
    }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  url = `http://localhost:${server.address().port}`;
});
test.afterAll(async () => { await new Promise(resolve => server.close(resolve)); });

async function ready(page) {
  await expect.poll(() => page.locator('video').evaluate(v => v.readyState)).toBeGreaterThanOrEqual(2);
}
test('running browser reports the pinned Chromium release', async ({browser}) => {
  expect(browser.version()).toBe(chromiumVersion);
});

test('hover button works through custom video controls and leaves with pointer', async ({page}) => {
  await page.goto(url + '/video'); await ready(page);
  const toggle = page.locator('mori-video-pip');
  await expect(toggle).toHaveCount(1);
  await expect(toggle).toBeHidden();
  await page.mouse.move(300, 160);
  await expect(toggle).toBeVisible();
  await page.locator('#outside').hover();
  await expect(toggle).toBeHidden();
});

test('SPA-style video replacement does not require navigation or duplicate controls', async ({page}) => {
  await page.goto(url + '/video?dynamic');
  await page.evaluate(() => addVideo()); await ready(page);
  await page.mouse.move(200, 150);
  await expect(page.locator('mori-video-pip')).toBeVisible();
  await page.evaluate(async () => {
    const video = document.querySelector('video');
    video.srcObject.getTracks().forEach(t => t.stop()); video.remove();
    await addVideo();
  });
  await ready(page);
  await page.mouse.move(210, 160);
  await expect(page.locator('mori-video-pip')).toHaveCount(1);
  await expect(page.locator('mori-video-pip')).toBeVisible();
});

test('ineligible and site-disabled videos do not show a toggle', async ({page}) => {
  await page.goto(url + '/video'); await ready(page);
  await page.locator('video').evaluate(v => { v.disablePictureInPicture = true; });
  await page.mouse.move(300, 160);
  await expect(page.locator('mori-video-pip')).toBeHidden();
  await page.locator('video').evaluate(v => { v.disablePictureInPicture = false; v.width = 100; v.height = 60; });
  await page.mouse.move(50, 30);
  await expect(page.locator('mori-video-pip')).toBeHidden();
});

test('embedded video has its own hover toggle', async ({page}) => {
  await page.goto(url + '/embed');
  const frame = page.frames().find(f => f.url().includes('127.0.0.1'));
  await ready(frame);
  await frame.locator('#controls').hover();
  await expect(frame.locator('mori-video-pip')).toBeVisible();
});

test('hover button opens and closes Chromium video PiP', async ({page}) => {
  await page.goto(url + '/video'); await ready(page);
  await page.mouse.move(300, 160);
  await page.locator('mori-video-pip').click();
  await expect.poll(() => page.evaluate(() => !!document.pictureInPictureElement)).toBe(true);
  await page.evaluate(() => document.exitPictureInPicture());
  await expect.poll(() => page.evaluate(() => !!document.pictureInPictureElement)).toBe(false);
});

test('MediaSource video with custom controls can enter PiP', async ({page}) => {
  await page.goto(url + '/video?dynamic');
  await page.evaluate(async () => {
    const media = new MediaSource();
    const video = document.createElement('video');
    video.width = 640; video.height = 360; video.muted = true;
    video.src = URL.createObjectURL(media);
    document.querySelector('#player').prepend(video);
    await new Promise(resolve => media.addEventListener('sourceopen', resolve, {once: true}));
    const buffer = media.addSourceBuffer('video/webm; codecs="vp8,vorbis"');
    const bytes = await (await fetch('/sample.webm')).arrayBuffer();
    const appended = new Promise(resolve => buffer.addEventListener('updateend', resolve, {once: true}));
    buffer.appendBuffer(bytes); await appended; media.endOfStream();
    await video.play();
  });
  await ready(page);
  await page.mouse.move(300, 160);
  await page.locator('mori-video-pip').click();
  await expect.poll(() => page.evaluate(() => !!document.pictureInPictureElement)).toBe(true);
  await page.evaluate(() => document.exitPictureInPicture());
});

for (const returnToTab of [true, false]) {
test(`automatic PiP follows tab switching; return to tab = ${returnToTab}`, async () => {
  const profile = await fs.mkdtemp(path.join(os.tmpdir(), 'mori-pip-auto-test-'));
  await fs.mkdir(path.join(profile, 'Default'));
  await fs.writeFile(path.join(profile, 'Default', 'Preferences'), JSON.stringify({
    mori: {picture_in_picture: {on_tab_switch: true, return_to_tab: returnToTab}},
  }));
  const context = await chromium.launchPersistentContext(profile, {
    executablePath: browserExecutable,
    headless: true,
    chromiumSandbox: true,
  });
  try {
    const page = await context.newPage();
    await page.goto(url + '/native'); await ready(page);
    await page.locator('#play').click();
    await expect.poll(() => page.locator('video').evaluate(v => v.currentTime)).toBeGreaterThan(0.5);
    const [other] = await Promise.all([
      context.waitForEvent('page'), page.locator('#other').click(),
    ]);
    await other.waitForLoadState(); await other.bringToFront();
    await expect.poll(() => page.evaluate(() => !!document.pictureInPictureElement)).toBe(true);
    await page.bringToFront();
    await expect.poll(() => page.evaluate(() => !!document.pictureInPictureElement)).toBe(!returnToTab);
    if (!returnToTab) await page.evaluate(() => document.exitPictureInPicture());
  } finally {
    await context.close();
  }
});

}

for (const [name, pref] of [['disabled', {enabled: false}], ['hover hidden', {hover_button: false}]]) {
  test(`saved ${name} setting is honored`, async ({}) => {
    const profile = await fs.mkdtemp(path.join(os.tmpdir(), 'mori-pip-test-'));
    await fs.mkdir(path.join(profile, 'Default'));
    await fs.writeFile(path.join(profile, 'Default', 'Preferences'), JSON.stringify({
      mori: {picture_in_picture: pref},
    }));
    const context = await chromium.launchPersistentContext(profile, {
      executablePath: browserExecutable,
      headless: true,
      chromiumSandbox: true,
    });
    try {
      const page = await context.newPage();
      await page.goto(url + '/video'); await ready(page);
      await page.mouse.move(300, 160);
      await expect(page.locator('mori-video-pip')).toHaveCount(0);
      if (name === 'disabled') {
        await page.evaluate(() => {
          const button = document.createElement('button'); button.id = 'try-pip';
          button.textContent = 'Try PiP';
          button.onclick = async () => {
            try { await document.querySelector('video').requestPictureInPicture();
              window.pipResult = 'entered';
            } catch (error) { window.pipResult = error.name; }
          };
          document.body.append(button);
        });
        await page.locator('#try-pip').click();
        await expect.poll(() => page.evaluate(() => window.pipResult)).toBeTruthy();
        expect(await page.evaluate(() => window.pipResult)).not.toBe('entered');
        expect(await page.evaluate(() => !!document.pictureInPictureElement)).toBe(false);
      }
    } finally {
      await context.close();
      // Keep isolated profiles in the OS temp folder for failure diagnostics.
    }
  });
}
