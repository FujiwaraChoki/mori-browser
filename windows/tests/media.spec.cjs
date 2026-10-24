// Run with Playwright's CLI against Mori, using isolated temporary profiles.
const {test, expect} = require('playwright/test');
const http = require('node:http');
const fs = require('node:fs/promises');
const path = require('node:path');
const {default_checkout} = require('../chromium.json');

const mp4Type = 'video/mp4; codecs="avc1.64001E,mp4a.40.2"';
let server, url;

test.beforeAll(async () => {
  // Exercise progressive URL playback and fragmented MediaSource separately.
  // The fragmented fixture's AAC fails in the URL demuxer in stock Chrome too.
  const progressive = await fs.readFile(path.join(default_checkout,
    'src/media/test/data/bear-1280x720.mp4'));
  const fragmented = await fs.readFile(path.join(default_checkout,
    'src/media/test/data/bear-640x360-av_frag.mp4'));
  server = http.createServer((req, res) => {
    if (req.url === '/sample.mp4' || req.url === '/fragmented.mp4') {
      const sample = req.url === '/sample.mp4' ? progressive : fragmented;
      res.writeHead(200, {'Content-Type': 'video/mp4', 'Content-Length': sample.length});
      res.end(sample);
    } else {
      res.writeHead(200, {'Content-Type': 'text/html'});
      res.end('<video muted playsinline width="640" height="360"></video>');
    }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  url = `http://127.0.0.1:${server.address().port}`;
});

test.afterAll(async () => {
  if (server) await new Promise(resolve => server.close(resolve));
});

test('H.264 and AAC are available to HTML video and MediaSource', async ({page}) => {
  await page.goto(url);
  for (const type of ['video/mp4; codecs="avc1.64001E"',
                      'audio/mp4; codecs="mp4a.40.2"', mp4Type]) {
    const support = await page.evaluate(type => ({
      html: document.createElement('video').canPlayType(type),
      mse: MediaSource.isTypeSupported(type),
    }), type);
    expect(support, type).toEqual({html: 'probably', mse: true});
  }
});

async function expectPlayback(page, width = 640) {
  await expect.poll(() => page.locator('video').evaluate(video => ({
    error: video.error?.message ?? null,
    advanced: video.currentTime > 0.2,
    decoded: video.getVideoPlaybackQuality().totalVideoFrames > 0,
    width: video.videoWidth,
  }))).toEqual({error: null, advanced: true, decoded: true, width});
}

test('an H.264/AAC MP4 URL decodes and plays', async ({page}) => {
  await page.goto(url);
  await page.locator('video').evaluate(async video => {
    video.src = '/sample.mp4';
    await video.play();
  });
  await expectPlayback(page, 1280);
});

test('an H.264/AAC MediaSource stream decodes and plays', async ({page}) => {
  await page.goto(url);
  await page.evaluate(async type => {
    const video = document.querySelector('video');
    const media = new MediaSource();
    video.src = URL.createObjectURL(media);
    await new Promise(resolve => media.addEventListener('sourceopen', resolve, {once: true}));
    const buffer = media.addSourceBuffer(type);
    const bytes = await (await fetch('/fragmented.mp4')).arrayBuffer();
    await new Promise((resolve, reject) => {
      buffer.addEventListener('updateend', resolve, {once: true});
      buffer.addEventListener('error', () => reject(new Error('MP4 append failed')), {once: true});
      buffer.appendBuffer(bytes);
    });
    media.endOfStream();
    await video.play();
  }, mp4Type);
  await expectPlayback(page);
});
