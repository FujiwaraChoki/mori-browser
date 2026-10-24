#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_PICTURE_IN_PICTURE_SCRIPT_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_PICTURE_IN_PICTURE_SCRIPT_H_

namespace mori {
// Runs in Chrome's isolated world, once per document (including video embeds).
// No site globals, player internals, or YouTube-specific selectors are needed.
inline constexpr char kPictureInPictureScript[] = R"MORI_PIP(
(() => {
  window.__moriVideoPip?.dispose();
  if (!window.__moriVideoPipEnabled || !document.pictureInPictureEnabled ||
      !document.documentElement)
    return;
  const host = document.createElement('mori-video-pip');
  const root = host.attachShadow({mode: 'closed'});
  const style = document.createElement('style');
  style.textContent = `
    :host { all: initial; position: fixed !important;
      z-index: 2147483647 !important; display: none !important; }
    :host([data-visible]) { display: block !important; }
    button { all: initial; display: flex; align-items: center; gap: 7px;
      box-sizing: border-box; padding: 8px 10px; border-radius: 8px;
      background: #242426ee; color: white; border: 1px solid #ffffff40;
      font: 500 15px/20px system-ui, sans-serif; cursor: pointer;
      box-shadow: 0 2px 8px #0005; }
    button:hover, button:focus-visible { background: #39393d; outline: 2px solid white; }
    svg { width: 20px; height: 20px; fill: none; stroke: currentColor; stroke-width: 1.8; }
  `;
  const button = document.createElement('button');
  button.type = 'button';
  button.title = 'Picture in Picture';
  button.setAttribute('aria-label', 'Picture in Picture');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('aria-hidden', 'true');
  for (const [x, y, width, height] of [[3, 4, 18, 16], [12, 11, 7, 7]]) {
    const rect = document.createElementNS(svg.namespaceURI, 'rect');
    for (const [key, value] of Object.entries({x, y, width, height, rx: 1.5}))
      rect.setAttribute(key, value);
    svg.append(rect);
  }
  button.append(svg);
  root.append(style, button);
  document.documentElement.append(host);
  let video = null;
  let idleTimer;
  const hide = () => host.removeAttribute('data-visible');
  const eligible = v => v.isConnected && v.readyState >= 2 &&
    v.videoWidth > 0 && !v.disablePictureInPicture;
  const place = () => {
    if (!video || !eligible(video) || document.fullscreenElement || document.hidden) {
      hide();
      return;
    }
    const rect = video.getBoundingClientRect();
    if (rect.width < 160 || rect.height < 90 || rect.bottom < 0 || rect.top > innerHeight) {
      hide();
      return;
    }
    host.style.setProperty('left', Math.max(8, Math.min(innerWidth - 64, rect.right - 64)) + 'px', 'important');
    host.style.setProperty('top', Math.max(8, rect.top + 12) + 'px', 'important');
    const exiting = document.pictureInPictureElement === video;
    button.title = exiting ? 'Return video to tab' : 'Picture in Picture';
    button.setAttribute('aria-label', button.title);
    host.setAttribute('data-visible', '');
  };
  const move = event => {
    if (event.composedPath().includes(host)) {
      clearTimeout(idleTimer);
      place();
      return;
    }
    video = [...document.querySelectorAll('video')].filter(eligible).find(v => {
      const r = v.getBoundingClientRect();
      return event.clientX >= r.left && event.clientX <= r.right &&
        event.clientY >= r.top && event.clientY <= r.bottom;
    }) || null;
    clearTimeout(idleTimer);
    place();
    idleTimer = setTimeout(() => { video = null; hide(); }, 2500);
  };
  const click = async event => {
    event.preventDefault();
    event.stopPropagation();
    if (!event.isTrusted || !video || !eligible(video)) return;
    try {
      if (document.pictureInPictureElement === video)
        await document.exitPictureInPicture();
      else
        await video.requestPictureInPicture();
      place();
    } catch {
      button.title = 'Picture in Picture is unavailable for this video';
      button.setAttribute('aria-label', button.title);
    }
  };
  const leave = () => { video = null; hide(); };
  document.addEventListener('mousemove', move, true);
  document.addEventListener('mouseleave', leave);
  document.addEventListener('scroll', place, true);
  document.addEventListener('fullscreenchange', leave, true);
  document.addEventListener('visibilitychange', leave, true);
  window.addEventListener('resize', place);
  button.addEventListener('click', click);
  window.__moriVideoPip = {dispose() {
    clearTimeout(idleTimer);
    document.removeEventListener('mousemove', move, true);
    document.removeEventListener('mouseleave', leave);
    document.removeEventListener('scroll', place, true);
    document.removeEventListener('fullscreenchange', leave, true);
    document.removeEventListener('visibilitychange', leave, true);
    window.removeEventListener('resize', place);
    host.remove();
    delete window.__moriVideoPip;
  }};
})();
)MORI_PIP";
}  // namespace mori

#endif
