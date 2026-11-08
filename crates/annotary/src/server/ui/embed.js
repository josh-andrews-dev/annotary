/* annotary embed core, protocol 1 (M3 of the Hermes plan; docs/dev/web-document-view.md,
   "Embed mode"). One classic script, inlined in the shell (`GET /embed/shell`) and in the head
   of every embed render, before app.js: it is the only code that talks to the embedder.

     frame → parent  {type:'annotary-ready', protocol}               the shell is listening
     parent → frame  {type:'annotary-init', path}                    show this page (again at any time)
     frame → parent  {type:'annotary-req', id, method, path, headers, body, binary?}
     parent → frame  {type:'annotary-res', id, status, headers, body, encoding?}   status 0 = unreachable
     parent → frame  {type:'annotary-event', payload}                something changed: look now
     frame → parent  {type:'annotary-nav', path}                     the frame now shows this page
     frame → parent  {type:'annotary-open-external', path | url}     open outside the frame

   It runs only in a frame whose origin is opaque (a sandboxed frame without allow-same-origin):
   anywhere else it defines nothing, and the page is an ordinary page. Every message it accepts
   comes from `window.parent`; any other type is ignored. A GET answer larger than one message may
   be is sent in parts (206 + `X-Annotary-Parts` / `X-Annotary-Etag`, `embed.rs`), which
   `request` joins. A page is shown by writing it into this document; the new page's copy of this
   script takes over (`gen`), and the old one's listeners and timers stand down. */
(function () {
  'use strict';
  if (String(window.origin || location.origin) !== 'null' || !window.parent || window.parent === window) return;
  var gen = window.__cdEmbedGen = (window.__cdEmbedGen || 0) + 1;
  function current() { return window.__cdEmbedGen === gen; }
  var P = window.parent, n = 0, waiting = {}, eventFns = [], TIMEOUT_MS = 30000, PART_TRIES = 3;
  var base = { 'X-Annotary-Embed': '1' };
  try {
    var zo = new Intl.DateTimeFormat(undefined, { hour: 'numeric' }).resolvedOptions();
    var cy = zo.hourCycle || (zo.hour12 ? 'h12' : 'h23');
    if (zo.timeZone) base['X-Annotary-Tz'] = zo.timeZone;
    base['X-Annotary-Hc'] = (cy === 'h23' || cy === 'h24') ? '24' : '12';
  } catch (e) { /* times stay relative */ }
  function post(m) { if (current()) P.postMessage(m, '*'); }

  /* ---- one message out, one back ---- */
  function once(method, path, headers, body, binary) {
    return new Promise(function (resolve, reject) {
      var id = 'cd' + gen + '-' + (++n);
      var hs = {};
      Object.keys(base).forEach(function (k) { hs[k] = base[k]; });
      Object.keys(headers || {}).forEach(function (k) { hs[k] = headers[k]; });
      waiting[id] = {
        done: function (m) { if (!+m.status) reject(new Error(m.error || 'could not reach the server')); else resolve(m); },
        timer: setTimeout(function () { delete waiting[id]; reject(new Error('no answer from the embedder')); }, TIMEOUT_MS)
      };
      var msg = { type: 'annotary-req', id: id, method: method, path: path, headers: hs, body: body === undefined ? null : body };
      if (binary) msg.binary = true;
      post(msg);
    });
  }
  function hdr(m, k) {
    var h = m.headers || {};
    for (var key in h) if (Object.prototype.hasOwnProperty.call(h, key) && key.toLowerCase() === k) return String(h[key]);
    return null;
  }
  function bytesOf(m) {
    var b = m.body;
    if (typeof b === 'string') {
      if (m.encoding !== 'base64') return new TextEncoder().encode(b);
      var s = atob(b), u = new Uint8Array(s.length);
      for (var i = 0; i < s.length; i++) u[i] = s.charCodeAt(i);
      return u;
    }
    if (b instanceof ArrayBuffer) return new Uint8Array(b);
    if (b && ArrayBuffer.isView(b)) return new Uint8Array(b.buffer, b.byteOffset, b.byteLength);
    return new Uint8Array(0);
  }
  /* What `fetch` resolves to, as far as the pages use it. */
  function response(status, headers, body) {
    var type = headers['content-type'] || '';
    function blob() { return Promise.resolve(body instanceof Blob ? body : new Blob([body], { type: type })); }
    function text() {
      if (typeof body === 'string') return Promise.resolve(body);
      return blob().then(function (b) { return b.text(); });
    }
    return {
      ok: status >= 200 && status < 300, status: status,
      headers: { get: function (k) { return headers[String(k).toLowerCase()] || null; } },
      text: text, blob: blob,
      json: function () { return text().then(function (t) { return JSON.parse(t); }); }
    };
  }
  function lower(m) {
    var out = {};
    Object.keys(m.headers || {}).forEach(function (k) { out[k.toLowerCase()] = String(m.headers[k]); });
    return out;
  }
  /* A request through the embedder, its parts joined: resolves to a fetch-like response, rejects
     when the server cannot be reached. */
  function request(method, path, o, tries) {
    o = o || {};
    return once(method, path, o.headers, o.body, o.binary).then(function (first) {
      var parts = +hdr(first, 'x-annotary-parts') || 0;
      if (+first.status !== 206 || parts < 2) {
        var b = first.body;
        var plain = typeof b === 'string' && first.encoding !== 'base64';
        return response(+first.status, lower(first), plain ? b : (b instanceof Blob ? b : bytesOf(first)));
      }
      var etag = hdr(first, 'x-annotary-etag'), got = [first];
      var step = Promise.resolve();
      for (var i = 1; i < parts; i++) {
        (function (k) {
          step = step.then(function () {
            var hs = {};
            Object.keys(o.headers || {}).forEach(function (h) { hs[h] = o.headers[h]; });
            hs['X-Annotary-Part'] = String(k);
            hs['X-Annotary-Etag'] = etag;
            return once(method, path, hs, null, o.binary).then(function (m) {
              if (+m.status !== 206) throw new Error('changed');
              got.push(m);
            });
          });
        })(i);
      }
      return step.then(function () {
        var headers = lower(first);
        var status = +hdr(first, 'x-annotary-status') || 200;
        if (got.every(function (m) { return typeof m.body === 'string' && m.encoding !== 'base64'; })) {
          return response(status, headers, got.map(function (m) { return m.body; }).join(''));
        }
        var chunks = got.map(bytesOf), len = 0, at = 0;
        chunks.forEach(function (c) { len += c.length; });
        var all = new Uint8Array(len);
        chunks.forEach(function (c) { all.set(c, at); at += c.length; });
        return response(status, headers, all);
      }, function (e) {
        // The answer changed between parts (a new comment): start over, a few times.
        if (e && e.message === 'changed' && (tries || 0) < PART_TRIES) return request(method, path, o, (tries || 0) + 1);
        throw e;
      });
    });
  }

  /* ---- pages ---- */
  function clean(p) {
    p = String(p || '');
    return p.charAt(0) === '/' && p.charAt(1) !== '/' ? p : null;
  }
  function here() { return window.__cdEmbedPath ? window.__cdEmbedPath + (window.__cdEmbedHash || '') : null; }
  var loading = null;
  function fail(msg) {
    var t = document.getElementById('cd-toast');
    if (t) { t.textContent = msg; t.hidden = false; } else if (document.body) document.body.textContent = msg;
  }
  /* Show the page at `path` (`/d/ID#t3`) in this frame: fetched through the embedder, written
     over this document; a redirect is followed (three at most). */
  function load(path, hops) {
    path = clean(path);
    if (!path || !current()) return;
    var cut = path.indexOf('#');
    var target = cut < 0 ? path : path.slice(0, cut), hash = cut < 0 ? '' : path.slice(cut);
    loading = path;
    request('GET', target || '/', { headers: { Accept: 'text/html' } }).then(function (r) {
      if (!current() || loading !== path) return null;
      var to = r.headers.get('location');
      if (r.status >= 300 && r.status < 400 && to && (hops || 0) < 3) { load(to + (to.indexOf('#') < 0 ? hash : ''), (hops || 0) + 1); return null; }
      return r.text().then(function (html) {
        if (!current() || loading !== path) return;
        window.__cdEmbedPath = target || '/';
        window.__cdEmbedHash = hash;
        document.open();
        document.write(html);
        document.close();
      });
    }).catch(function () { if (current() && loading === path) fail('Annotary could not be reached.'); });
  }
  function initPath(m) {
    if (typeof m.path === 'string') return clean(m.path);
    if (typeof m.doc === 'string' && /^[A-Za-z0-9_-]+$/.test(m.doc)) {
      return '/d/' + m.doc + (typeof m.thread === 'string' && /^t\d+$/.test(m.thread) ? '#' + m.thread : '');
    }
    return null;
  }
  window.addEventListener('message', function (e) {
    if (!current() || e.source !== P) return;
    var m = e.data;
    if (!m || typeof m !== 'object') return;
    if (m.type === 'annotary-res') {
      var w = waiting[m.id];
      if (!w) return;
      delete waiting[m.id];
      clearTimeout(w.timer);
      w.done(m);
    } else if (m.type === 'annotary-event') {
      eventFns.slice().forEach(function (fn) { fn(m.payload || m.event || null); });
    } else if (m.type === 'annotary-init') {
      var p = initPath(m);
      if (p && p !== here() && p !== loading) load(p);
    }
  });

  window.AnnotaryEmbed = {
    protocol: 1,
    current: current,
    request: request,
    /* Go to a page of this server: tell the embedder where the frame now is, then show it. */
    navigate: function (path) {
      path = clean(path);
      if (!path || !current()) return;
      post({ type: 'annotary-nav', path: path });
      load(path);
    },
    openExternal: function (target) {
      if (clean(target)) post({ type: 'annotary-open-external', path: target });
      else if (/^(https?:|mailto:)/i.test(target)) post({ type: 'annotary-open-external', url: target });
    },
    /* `fn` runs when the embedder says something changed (the payload, when it sends one). */
    onEvent: function (fn) { eventFns.push(fn); },
    path: function () { return window.__cdEmbedPath || '/'; },
    hash: function () { return window.__cdEmbedHash || ''; },
    zone: function () { return base['X-Annotary-Tz'] || null; },
    /* The shell: ask for the first page; the path is the embedder's `annotary-init`, else the
       shell URL's fragment, else the list (after a moment with no init). */
    boot: function () {
      post({ type: 'annotary-ready', protocol: 1 });
      var frag = null;
      try { frag = clean(decodeURIComponent((location.hash || '').slice(1))); } catch (e) { frag = null; }
      if (frag) load(frag);
      else setTimeout(function () { if (!loading) load('/'); }, 1500);
    }
  };
})();
