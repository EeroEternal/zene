/* ═══════════════════════════════════════════════════════════════
   zene.sh — Minimal interactive logic
   ═══════════════════════════════════════════════════════════════ */

(function() {
  'use strict';

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }

  function init() {
    initTheme();
    initCopy();
  }

  /* ── Theme ────────────────────────────────────────────────── */
  function initTheme() {
    var btn = document.getElementById('themeToggle');
    var html = document.documentElement;
    var saved = localStorage.getItem('zene-theme') || 'dark';
    html.setAttribute('data-theme', saved);
    btn.textContent = saved === 'dark' ? '◐' : '◑';

    btn.addEventListener('click', function() {
      var current = html.getAttribute('data-theme');
      var next = current === 'dark' ? 'light' : 'dark';
      html.setAttribute('data-theme', next);
      localStorage.setItem('zene-theme', next);
      btn.textContent = next === 'dark' ? '◐' : '◑';
    });
  }

  /* ── Copy Button ──────────────────────────────────────────── */
  function initCopy() {
    var buttons = document.querySelectorAll('.copy-btn');
    for (var i = 0; i < buttons.length; i++) {
      (function(btn) {
        btn.addEventListener('click', function() {
          var text = btn.getAttribute('data-copy');

          if (navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(text).then(function() {
              showCopied(btn);
            });
          } else {
            var ta = document.createElement('textarea');
            ta.value = text;
            ta.style.position = 'fixed';
            ta.style.opacity = '0';
            document.body.appendChild(ta);
            ta.select();
            document.execCommand('copy');
            document.body.removeChild(ta);
            showCopied(btn);
          }
        });
      })(buttons[i]);
    }
  }

  function showCopied(btn) {
    var original = btn.textContent;
    btn.textContent = 'Copied!';
    btn.style.color = 'var(--accent)';
    setTimeout(function() {
      btn.textContent = original;
      btn.style.color = '';
    }, 1500);
  }

})();
