/* The Version selects of the diff page (`compare.rs`). A classic script placed before the selects,
   and so running before they exist: no moment passes in which one is on screen and a change to it
   is lost. `app.js` waits for a frame after the first paint, far too late for this. The listener
   is on `document` because the selects do not exist yet. In an embed render the frame cannot leave
   by URL, so it goes through the embedder (`embed.js`, in the head, has already run). */
(function () {
  'use strict';
  var E = window.AnnotaryEmbed;
  document.addEventListener('change', function (e) {
    var t = e.target;
    if (!t || (t.id !== 'cd-dxfrom' && t.id !== 'cd-dxto') || (E && !E.current())) return;
    var from = document.getElementById('cd-dxfrom'), to = document.getElementById('cd-dxto');
    var path = t.form.getAttribute('action') + '?from=' + encodeURIComponent(from.value) + '&to=' + encodeURIComponent(to.value);
    if (E && document.querySelector('.cd-app.cd-embed')) E.navigate(path);
    else location.href = path;
  });
})();
