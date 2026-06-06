// JavaScript agent injected into every frame to power the sidebar media player
// and automatic Picture-in-Picture.
//
//  • Detects the "primary" media element once it looks like a real playback
//    session (playing with audio, or directly selected by the user).
//  • Reports state changes to native via a console channel
//    (`console.debug('__MORI_MEDIA__' + json)`), captured in
//    BrowserClient::OnConsoleMessage.
//  • Exposes `window.__moriMedia(action, value)` for native-driven controls.
//  • Auto-enters PiP when the tab is hidden (if `window.__moriAutoPiP`).
#pragma once

static const char kMoriMediaAgent[] = R"JS(
(function(){
  if (window.__moriMediaInstalled) { return; }
  window.__moriMediaInstalled = true;
  if (typeof window.__moriAutoPiP === 'undefined') { window.__moriAutoPiP = false; }

  var last = "";
  var recentMediaGestureUntil = 0;

  function closestElement(target, selector){
    for (var n = target; n; n = n.parentNode || (n.host || null)) {
      if (n.nodeType === 1 && n.matches && n.matches(selector)) { return n; }
    }
    return null;
  }

  function mediaFromTarget(target){
    for (var n = target; n; n = n.parentNode || (n.host || null)) {
      if (n.nodeType === 1 && (n.tagName === 'VIDEO' || n.tagName === 'AUDIO')) { return n; }
    }
    return null;
  }

  function hasAudibleTrack(el){
    return !el.muted && (typeof el.volume !== 'number' || el.volume > 0);
  }

  function isYouTubePreview(el){
    var host = location.hostname.replace(/^www\./,'');
    if (host !== 'youtube.com' && host !== 'm.youtube.com') { return false; }
    if (!el.muted) { return false; }
    if (closestElement(el, '#movie_player, ytd-player, #shorts-player')) { return false; }
    return true;
  }

  function mediaFromPlayerTarget(target){
    var player = closestElement(target, '#movie_player, ytd-player, #shorts-player');
    return player ? player.querySelector('video,audio') : null;
  }

  function eligible(el){
    if (!el) { return false; }
    if (isYouTubePreview(el)) { return false; }
    if (el.__moriMediaEligible || el.__moriMediaUserSelected) { return true; }
    if (!el.paused && hasAudibleTrack(el)) { return true; }
    return !el.paused && Date.now() < recentMediaGestureUntil;
  }

  function markIfEligible(el){
    if (eligible(el)) { el.__moriMediaEligible = true; }
  }

  ['pointerdown','click','keydown','touchstart'].forEach(function(ev){
    document.addEventListener(ev, function(e){
      var el = mediaFromTarget(e.target) || mediaFromPlayerTarget(e.target);
      if (el) {
        recentMediaGestureUntil = Date.now() + 4000;
        el.__moriMediaUserSelected = true;
        markIfEligible(el);
      }
    }, true);
  });

  function pick(){
    var els = Array.prototype.slice.call(document.querySelectorAll('video,audio'));
    els.forEach(markIfEligible);
    els = els.filter(function(m){
      return (m.currentSrc || m.src) && (m.__moriMediaEligible || eligible(m));
    });
    if (!els.length) { return null; }
    els.sort(function(a,b){
      var ap = a.paused ? 0 : 1, bp = b.paused ? 0 : 1;
      if (ap !== bp) { return bp - ap; }
      var aa = (a.videoWidth||0)*(a.videoHeight||0);
      var ba = (b.videoWidth||0)*(b.videoHeight||0);
      return ba - aa;
    });
    return els[0];
  }

  function meta(){
    try {
      var m = navigator.mediaSession && navigator.mediaSession.metadata;
      if (m) {
        var art = (m.artwork && m.artwork.length) ? m.artwork[m.artwork.length-1].src : '';
        return { title: m.title || '', artist: m.artist || '', artwork: art };
      }
    } catch(e){}
    return null;
  }

  function report(force){
    var el = pick();
    var state;
    if (!el) {
      state = { hasMedia:false };
    } else {
      // Native auto-PiP: when enabled, the engine pops this video out on tab
      // hide (no user-gesture restriction). Kept in sync with the setting.
      try { el.autoPictureInPicture = !!window.__moriAutoPiP; } catch(e){}
      var md = meta();
      state = {
        hasMedia: true,
        playing: !el.paused,
        title: (md && md.title) || document.title || '',
        artist: (md && md.artist) || location.hostname.replace(/^www\./,''),
        artwork: (md && md.artwork) || '',
        position: el.currentTime || 0,
        duration: (isFinite(el.duration) ? el.duration : 0),
        muted: !!el.muted,
        isVideo: el.tagName === 'VIDEO',
        inPiP: (document.pictureInPictureElement === el),
        canPiP: el.tagName === 'VIDEO' && !!document.pictureInPictureEnabled
      };
    }
    var s = JSON.stringify(state);
    if (force || s !== last) { last = s; console.debug('__MORI_MEDIA__' + s); }
  }

  function pip(el){
    try {
      if (document.pictureInPictureElement) { document.exitPictureInPicture(); }
      else if (el && el.requestPictureInPicture) { el.requestPictureInPicture().catch(function(){}); }
    } catch(e){}
  }

  window.__moriMedia = function(action, value){
    var el = pick();
    if (!el && action !== 'pip') { return; }
    switch(action){
      case 'play':   el && el.play(); break;
      case 'pause':  el && el.pause(); break;
      case 'toggle': el && (el.paused ? el.play() : el.pause()); break;
      case 'seek':   if (el) { el.currentTime = value; } break;
      case 'seekBy': if (el) { el.currentTime = Math.max(0, (el.currentTime||0) + value); } break;
      case 'mute':   if (el) { el.muted = !el.muted; } break;
      case 'pip':    pip(el); break;
    }
    setTimeout(function(){ report(true); }, 60);
  };

  document.addEventListener('visibilitychange', function(){
    if (document.hidden && window.__moriAutoPiP) {
      var el = pick();
      if (el && el.tagName === 'VIDEO' && !el.paused &&
          el.requestPictureInPicture && !document.pictureInPictureElement) {
        el.requestPictureInPicture().catch(function(){});
      }
    } else if (!document.hidden && document.pictureInPictureElement) {
      document.exitPictureInPicture().catch(function(){});
    }
  });

  ['play','pause','loadedmetadata','ratechange','volumechange','enterpictureinpicture','leavepictureinpicture']
    .forEach(function(ev){ document.addEventListener(ev, function(){ report(true); }, true); });

  setInterval(function(){ report(false); }, 1000);
  report(true);
})();
)JS";
