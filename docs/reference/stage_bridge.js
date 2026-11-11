// Bridge injected into the avatar stage. Two jobs:
//   1. Forward the pointer gestures Rust cannot see (the WebView2 child swallows them) back
//      through wry's IPC channel.
//   2. Keep the JS API surface defensively guarded so a stage that fails to load never throws.
(function () {
  if (window.__ariaBridgeInstalled) { return; }
  window.__ariaBridgeInstalled = true;

  var post = function (kind, payload) {
    try {
      window.ipc.postMessage(JSON.stringify({ kind: kind, data: payload || {} }));
    } catch (e) { /* the page may be loaded outside wry */ }
  };

  var dragStart = null;
  var lastX = 0;
  var lastY = 0;
  var lastDir = 0;
  var reversals = [];
  var headpatX = null;
  var headpatDir = 0;
  var headpatReversals = [];
  var lastHeadpat = 0;

  function now() { return Date.now() / 1000; }

  window.addEventListener('mousedown', function (e) {
    if (e.button === 0) {
      dragStart = { x: e.clientX, y: e.clientY };
      lastX = e.clientX;
      lastY = e.clientY;
      lastDir = 0;
      reversals = [];
      post('drag_start', { x: e.clientX, y: e.clientY });
    } else if (e.button === 1) {
      e.preventDefault();
      post('middle_click');
    }
  });

  window.addEventListener('mousemove', function (e) {
    // Head-pat detection: a deliberate back-and-forth scrub across the head band, never a single
    // sweep. Requires four direction reversals within 1.25 s and rate-limits to one every 5 s.
    var inHeadBand = e.clientY < window.innerHeight * 0.4 &&
        Math.abs(e.clientX - window.innerWidth * 0.5) < window.innerWidth * 0.36;
    if (inHeadBand) {
      var t = now();
      if (headpatX !== null) {
        var step = e.clientX - headpatX;
        if (Math.abs(step) >= 8) {
          var dir = step > 0 ? 1 : -1;
          if (headpatDir !== 0 && dir !== headpatDir) {
            headpatReversals = headpatReversals.filter(function (x) { return t - x <= 1.25; });
            headpatReversals.push(t);
            if (headpatReversals.length >= 4 && t - lastHeadpat > 5.0) {
              headpatReversals = [];
              lastHeadpat = t;
              post('headpat');
            }
          }
          headpatDir = dir;
        }
      }
      headpatX = e.clientX;
    } else {
      headpatX = null;
      headpatDir = 0;
      headpatReversals = [];
    }

    if (!dragStart) { return; }
    var dx = e.clientX - dragStart.x;
    var dy = e.clientY - dragStart.y;
    if (Math.abs(dx) + Math.abs(dy) <= 5) { return; }

    post('drag_move', { x: e.clientX, y: e.clientY });

    var stepX = e.clientX - lastX;
    var stepY = e.clientY - lastY;
    post('drag_inertia', {
      x: Math.max(-1, Math.min(1, stepX / 16)),
      y: Math.max(-1, Math.min(1, stepY / 16))
    });

    if (Math.abs(stepX) >= 12) {
      var dirX = stepX > 0 ? 1 : -1;
      var t2 = now();
      if (lastDir !== 0 && dirX !== lastDir) {
        reversals = reversals.filter(function (x) { return t2 - x <= 0.85; });
        reversals.push(t2);
        if (reversals.length >= 5) {
          reversals = [];
          post('shake');
        }
      }
      lastDir = dirX;
    }
    lastX = e.clientX;
    lastY = e.clientY;
  });

  window.addEventListener('mouseup', function (e) {
    if (e.button === 0 && dragStart) {
      dragStart = null;
      post('drag_end');
    }
  });

  window.addEventListener('dblclick', function (e) {
    if (e.button === 0) { post('poke'); }
  });

  window.addEventListener('contextmenu', function (e) {
    e.preventDefault();
    post('context_menu');
  });

  document.addEventListener('dragenter', function (e) { e.preventDefault(); });
  document.addEventListener('dragover', function (e) { e.preventDefault(); });

  // Voice audio hooks: the stage owns the WebAudio graph so lip-sync can read the FFT.
  if (typeof window.playVoiceAudio !== 'function') {
    window.playVoiceAudio = function (url) {
      window.__pendingAudioUrl = url;
    };
  }
  if (typeof window.stopVoiceAudio !== 'function') {
    window.stopVoiceAudio = function () {
      window.__pendingAudioUrl = null;
    };
  }

  // Report readiness so Rust knows the model finished loading.
  var announce = function (label) { post('stage_ready', { label: String(label || '') }); };
  var originalMode = window.setAvatarMode;
  if (typeof originalMode === 'function') {
    window.setAvatarMode = function (mode, url) {
      var p = originalMode.call(window, mode, url);
      if (p && typeof p.then === 'function') { p.then(function () { announce(mode); }); }
      else { announce(mode); }
      return p;
    };
  }

  // __stagePlate: paint the stage itself rather than relying on child-window transparency.
  var plate = document.createElement('style');
  plate.textContent =
    'html,body{background:linear-gradient(180deg,#0a0b0d 0%,#121418 62%,#0d0f12 100%) !important;}' +
    '#vrm-container,#live2d-container{background:transparent !important;}';
  (document.head || document.documentElement).appendChild(plate);

  window.addEventListener('load', function () { announce('load'); });
})();
