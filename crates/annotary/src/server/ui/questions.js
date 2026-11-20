/* Questions (docs/dev/questions.md): the controls the server spliced into the page answer with
   PUT /api/v1/docs/{id}/questions/{key}/answer, optimistic with rollback; the live feed's
   `question.answered` re-reads GET …/questions. This script only flips pressed states, radios and
   the who-chose text; the markup is the server's (`server/qview.rs`), a card is redrawn with
   `cd.refreshCard`. Only on a page whose Version has Questions; every request goes through
   `cd.api` (the transport, so embed mode works). */
(function () {
  'use strict';
  var cd = window.cd;
  var D = cd && cd.data();
  if (!D || !D.questions) return;
  var docEl = document.getElementById('cd-doc');
  var Q = {};
  D.questions.forEach(function (q) { Q[q.key] = q; });
  var busy = {};
  function $$(sel, root) { return Array.prototype.slice.call((root || document).querySelectorAll(sel)); }
  function sel(key) { return '[data-q="' + key.replace(/"/g, '') + '"]'; }
  function mine(q) {
    for (var i = 0; i < q.answers.length; i++) if (q.answers[i].by === D.me) return q.answers[i].option;
    return null;
  }
  function label(q, o) {
    if (o === 'other') return 'Other';
    var f = q.options.filter(function (x) { return x.id === o; })[0];
    return (f && f.label) || o;
  }
  /* The same words as `qview::who_text`. */
  function who(q) {
    var groups = [];
    q.answers.forEach(function (a) {
      var h = '@' + a.by + (a.changed ? '*' : '');
      var g = groups.filter(function (x) { return x.o === a.option; })[0];
      if (g) g.h.push(h); else groups.push({ o: a.option, h: [h] });
    });
    if (!groups.length) return 'No answer yet';
    return groups.map(function (g) { return label(q, g.o) + ' ' + g.h.join(', '); }).join(' · ');
  }
  function paint(key) {
    var q = Q[key];
    if (!q) return;
    var m = mine(q);
    $$(sel(key)).forEach(function (el) {
      $$('.cd-qopt', el).forEach(function (b) { b.setAttribute('aria-pressed', String(b.getAttribute('data-o') === m)); });
      if (!q.decided) $$('.cd-qwho', el).forEach(function (w) { w.textContent = who(q); });
      if (el.classList.contains('cd-qradio')) el.checked = el.getAttribute('data-o') === m;
    });
    summary();
  }
  function summary() {
    var table = D.questions.map(function (q) { return Q[q.key]; }).filter(function (q) { return q.source === 'table'; });
    var done = table.filter(function (q) { return q.decided || mine(q); }).length;
    var k = pending().length;
    var s = document.getElementById('cd-qsum');
    if (s) s.textContent = done + ' of ' + table.length + ' answered';
    var b = document.getElementById('cd-qaccept');
    if (b) { b.textContent = 'Accept recommended (' + k + ')'; b.hidden = !k; }
  }
  function pending() {
    return D.questions.map(function (q) { return Q[q.key]; })
      .filter(function (q) { return q.source === 'table' && !q.decided && q.rec && !mine(q); });
  }
  function path(key) { return '/api/v1/docs/' + encodeURIComponent(D.id) + '/questions/' + encodeURIComponent(key) + '/answer'; }
  function answer(key, option) {
    var q = Q[key];
    var before = JSON.parse(JSON.stringify(q));
    q.answers = q.answers.filter(function (a) { return a.by !== D.me; });
    if (option) q.answers.push({ by: D.me, option: option, at: new Date().toISOString(), version: D.head, changed: false });
    paint(key);
    busy[key] = (busy[key] || 0) + 1;
    return cd.api('PUT', path(key), { option: option, version: D.head }).then(function (r) {
      busy[key]--;
      if (!r.ok) {
        Q[key] = before;
        paint(key);
        cd.toast('Answer to ' + key + ' not saved: ' + r.error);
        return false;
      }
      if (r.json && r.json.question) { Q[key] = r.json.question; paint(key); }
      if (Q[key].thread) cd.refreshCard(Q[key].thread);
      return true;
    });
  }
  /* Other… and Comment: the Question's Thread (a reply when it has one, else its first Comment). */
  function discuss(key, other) {
    var q = Q[key];
    if (other) answer(key, 'other');
    var card = q.thread && docEl.querySelector('.cd-thread[data-t="' + q.thread + '"]');
    if (card) {
      cd.select(q.thread, true);
      var reply = card.querySelector('[data-act="reply"]');
      if (reply) reply.click();
      return;
    }
    cd.openComposer({ kind: 'question', key: key, line: q.place ? q.place.a : q.row, done: function (tid) { Q[key].thread = tid; } });
  }
  function reload() {
    var v = D.version !== D.head ? '?version=' + D.version : '';
    cd.api('GET', '/api/v1/docs/' + encodeURIComponent(D.id) + '/questions' + v).then(function (r) {
      if (!r.ok || !r.json) return;
      r.json.questions.forEach(function (q) {
        if (busy[q.key]) return;
        Q[q.key] = q;
        paint(q.key);
      });
    });
  }
  window.cdq = function (ev) {
    var p = ev.payload || {};
    if (ev.type === 'question.answered') {
      if (ev.by === D.me && busy[p.key]) return;
      reload();
    } else if (ev.type === 'thread.created' && p.question && Q[p.question]) {
      Q[p.question].thread = p.thread;
    }
  };
  document.addEventListener('click', function (e) {
    var b = e.target.closest('.cd-qopt,.cd-qcmt,#cd-qaccept');
    if (!b || b.disabled) return;
    if (b.id === 'cd-qaccept') {
      e.preventDefault();
      pending().reduce(function (chain, q) { return chain.then(function () { return answer(q.key, q.rec); }); }, Promise.resolve());
      return;
    }
    var host = b.closest('[data-q]');
    var key = host && host.getAttribute('data-q');
    if (!key || !Q[key]) return;
    e.preventDefault();
    e.stopPropagation();
    if (b.classList.contains('cd-qcmt')) { discuss(key, false); return; }
    var o = b.getAttribute('data-o');
    if (o === 'other') { discuss(key, true); return; }
    answer(key, b.getAttribute('aria-pressed') === 'true' ? null : o);
  }, true);
  document.addEventListener('change', function (e) {
    var r = e.target;
    if (!r.classList || !r.classList.contains('cd-qradio') || !r.checked) return;
    var key = r.getAttribute('data-q');
    if (Q[key]) answer(key, r.getAttribute('data-o'));
  });
})();
