import Foundation

/// JavaScript injected into every page to power the sidebar media player and
/// Picture-in-Picture. Unlike the old CEF agent (which pushed state over a
/// `console.debug` channel), this overlay uses a pull model:
///
///   • `window.__moriMediaState()` returns a JSON snapshot of the page's
///     primary media element. `BrowserStore` polls it once a second and
///     rebroadcasts it as the `MoriMediaUpdated` notification that
///     `MediaController` already listens for.
///   • `window.__moriMedia(action, value)` performs a transport/PiP action.
///     Native callers run it through `ExecuteJavaScriptWithUserGestureForTests`
///     so `requestPictureInPicture()` (and autoplay) clear the user-activation
///     gate — which is why auto-PiP needs no Chromium feature flag here.
///
/// "Primary media" is whichever video/audio looks like a real playback session
/// (playing with audio, or directly clicked by the user), preferring the
/// largest playing video. The detection mirrors the original agent.
enum MediaAgentScripts {
    static let agent = #"""
    (function(){
      if (window.__moriMediaInstalled) { return; }
      window.__moriMediaInstalled = true;

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

      function pickVideo(){
        var el = pick();
        return (el && el.tagName === 'VIDEO') ? el : null;
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

      function buildState(){
        var el = pick();
        if (!el) { return { hasMedia:false }; }
        var md = meta();
        return {
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

      function pipToggle(){
        try {
          if (document.pictureInPictureElement) { document.exitPictureInPicture(); return; }
          var el = pickVideo();
          if (el && el.requestPictureInPicture) { el.requestPictureInPicture().catch(function(){}); }
        } catch(e){}
      }
      function pipEnter(){
        try {
          if (document.pictureInPictureElement) { return; }
          var el = pickVideo();
          if (el && el.requestPictureInPicture) { el.requestPictureInPicture().catch(function(){}); }
        } catch(e){}
      }
      function pipExit(){
        try {
          if (document.pictureInPictureElement) { document.exitPictureInPicture().catch(function(){}); }
        } catch(e){}
      }

      // Pull entry point: returns a JSON string snapshot, or '' on failure.
      window.__moriMediaState = function(){
        try { return JSON.stringify(buildState()); } catch(e){ return ''; }
      };

      // Command entry point, driven from native (with a synthetic user gesture).
      window.__moriMedia = function(action, value){
        if (action === 'pip')      { pipToggle(); return; }
        if (action === 'pipEnter') { pipEnter();  return; }
        if (action === 'pipExit')  { pipExit();   return; }
        var el = pick();
        if (!el) { return; }
        switch(action){
          case 'play':   if (el.play)  { el.play(); }  break;
          case 'pause':  if (el.pause) { el.pause(); } break;
          case 'toggle': el.paused ? (el.play && el.play()) : (el.pause && el.pause()); break;
          case 'seek':   el.currentTime = value; break;
          case 'seekBy': el.currentTime = Math.max(0, (el.currentTime||0) + value); break;
          case 'mute':   el.muted = !el.muted; break;
        }
      };
    })();
    """#
}
