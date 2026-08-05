/* rigid — site behaviour: theme, syntax highlighting, TOC, identity demo */
(function () {
  "use strict";

  // ── theme ──────────────────────────────────────────────────────────
  var root = document.documentElement;
  function setTheme(dark) {
    root.classList.toggle("dark", dark);
    try { localStorage.setItem("rigid-theme", dark ? "dark" : "light"); } catch (e) {}
  }
  var btn = document.getElementById("theme");
  if (btn) btn.addEventListener("click", function () { setTheme(!root.classList.contains("dark")); });

  // ── header page picker ─────────────────────────────────────────────
  var picker = document.getElementById("navpick");
  if (picker) {
    picker.addEventListener("change", function () {
      if (picker.value) location.href = picker.value;
    });
  }

  // ── syntax highlighting for rigid source ───────────────────────────
  var KW = { fn:1, let:1, return:1, while:1, if:1, else:1, import:1, from:1,
             table:1, dyn:1, memo:1, extern:1 };
  var BI = { print:1, len:1, push:1, input:1, fail:1, try:1, str:1, int:1,
             substr:1, chars:1, join:1, read_file:1, write_file:1 };

  function esc(s) {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }
  function span(cls, s) { return '<span class="' + cls + '">' + esc(s) + "</span>"; }

  function highlight(src) {
    var re = /(\/\/[^\n]*)|("(?:\\.|[^"\\])*")|(\b\d+\b)|(!io)|([A-Za-z_][A-Za-z0-9_]*)/g;
    var out = "", last = 0, m;
    while ((m = re.exec(src)) !== null) {
      out += esc(src.slice(last, m.index));
      if (m[1]) out += span("tok-com", m[1]);
      else if (m[2]) out += span("tok-str", m[2]);
      else if (m[3]) out += span("tok-num", m[3]);
      else if (m[4]) out += span("tok-eff", m[4]);
      else {
        var w = m[5];
        out += KW[w] ? span("tok-kw", w) : BI[w] ? span("tok-bi", w) : esc(w);
      }
      last = re.lastIndex;
    }
    return out + esc(src.slice(last));
  }

  document.querySelectorAll("code.rg").forEach(function (el) {
    el.innerHTML = highlight(el.textContent);
  });

  // ── copy buttons ───────────────────────────────────────────────────
  document.querySelectorAll("[data-copy]").forEach(function (b) {
    b.addEventListener("click", function () {
      var target = document.querySelector(b.getAttribute("data-copy"));
      if (!target) return;
      var text = target.innerText.replace(/^\$ /gm, "");
      function done() {
        var old = b.textContent;
        b.textContent = "copied";
        setTimeout(function () { b.textContent = old; }, 1300);
      }
      function legacy() {
        var ta = document.createElement("textarea");
        ta.value = text; document.body.appendChild(ta); ta.select();
        try { document.execCommand("copy"); done(); } catch (e) {}
        document.body.removeChild(ta);
      }
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(done, legacy);
      } else legacy();
    });
  });

  // ── table-of-contents highlighting (learn page) ────────────────────
  var tocLinks = [].slice.call(document.querySelectorAll(".toc a[href^='#']"));
  if (tocLinks.length) {
    var heads = tocLinks
      .map(function (a) { return document.getElementById(a.getAttribute("href").slice(1)); })
      .filter(Boolean);

    // The active section is the last heading whose top has passed the nav.
    // Computing from scroll position stays correct when jumping via a link,
    // which a pure-intersection approach does not.
    var ticking = false;
    function sync() {
      ticking = false;
      var current = heads.length ? heads[0].id : null;
      for (var i = 0; i < heads.length; i++) {
        if (heads[i].getBoundingClientRect().top <= 96) current = heads[i].id;
        else break;
      }
      tocLinks.forEach(function (a) {
        a.classList.toggle("on", a.getAttribute("href") === "#" + current);
      });
    }
    function onScroll() {
      if (ticking) return;
      ticking = true;
      requestAnimationFrame(sync);
    }
    addEventListener("scroll", onScroll, { passive: true });
    addEventListener("resize", onScroll);
    sync();
  }

  // ── identity demo (index) ──────────────────────────────────────────
  var codeEl = document.getElementById("demo-code");
  if (!codeEl) return;

  // Hashes below are real output from `rigid hash`.
  var NAMES = [
    { fn: "damage",       a: "hp",      b: "hit" },
    { fn: "take_hit",     a: "health",  b: "dmg" },
    { fn: "apply_damage", a: "current", b: "amount" }
  ];
  var BODIES = [
    { expr: function (a, b) { return a + " - " + b; },
      hash: "sha256:51e00ffda76426bb01d99f0fe5d5bb3ebedab8711a3be637ea54326def588b7f" },
    { expr: function (a, b) { return a + " - " + b + " * 2"; },
      hash: "sha256:7bafa60f0f823946be52e8f6dad43c1b5b7757d55cf604de1bc375667e7048fb" },
    { expr: function (a, b) { return a + " - " + b + " + 5"; },
      hash: "sha256:339057ab065e921341ca93225461d3ec30d2a45b7b8e0af0d89a3a1cd9b2d759" }
  ];
  var ni = 0, bi = 0;
  var hashEl = document.getElementById("demo-hash");
  var verdictEl = document.getElementById("demo-verdict");
  var noteEl = document.getElementById("demo-note");

  function render() {
    var n = NAMES[ni], b = BODIES[bi];
    codeEl.innerHTML = highlight(
      "fn " + n.fn + "(" + n.a + ", " + n.b + ") {\n  return " + b.expr(n.a, n.b) + ";\n}"
    );
    hashEl.textContent = b.hash;
    hashEl.classList.toggle("moved", bi !== 0);
    verdictEl.classList.toggle("moved", bi !== 0);
    verdictEl.textContent = bi !== 0 ? "identity moved" : "identity unchanged";
    noteEl.textContent = bi !== 0
      ? "The body changed, so the hash moved — whatever the function is called. Tests downstream of it "
        + "re-execute; everything else stays cached."
      : (ni !== 0
          ? "Renamed, and the hash is byte-for-byte the one above. This is why a renamed definition can be "
            + "found by identity and its call sites repaired automatically."
          : "The definition as first written.");
  }

  document.querySelectorAll(".ctlrow .chip").forEach(function (b) {
    b.addEventListener("click", function () {
      var isName = b.hasAttribute("data-name");
      var attr = isName ? "data-name" : "data-body";
      document.querySelectorAll("[" + attr + "]").forEach(function (o) {
        o.setAttribute("aria-pressed", o === b ? "true" : "false");
      });
      if (isName) ni = +b.getAttribute("data-name"); else bi = +b.getAttribute("data-body");
      render();
    });
  });
  render();
})();
