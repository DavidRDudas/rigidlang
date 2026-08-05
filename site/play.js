/* Playground UI: editor, examples, and the worker lifecycle. */
(function () {
  "use strict";

  var TIMEOUT_MS = 5000;

  var editor = document.getElementById("src");
  var gutter = document.getElementById("gutter");
  var out = document.getElementById("out");
  var entry = document.getElementById("entry");
  var argsBox = document.getElementById("args");
  var status = document.getElementById("status");
  var picker = document.getElementById("examples");
  var buttons = [].slice.call(document.querySelectorAll("[data-action]"));

  var worker = null, booted = false, pending = null, seq = 0, timer = null;

  // ── examples ─────────────────────────────────────────────────────────
  // Every one of these was run against the compiler before it landed here.
  var EXAMPLES = {
    "mandelbrot": [
      "// The Mandelbrot set in integer fixed-point. rigid has floats, but",
      "// fixed-point keeps every frame exact and the arithmetic honest.",
      "// Every number below is scaled by 1000. Run `draw`.",
      "fn scale() { return 1000; }",
      "",
      "fn fmul(a, b) { return (a * b) / scale(); }",
      "",
      "fn iter(zr, zi, cr, ci, n, cap) {",
      "  return if (n >= cap) cap",
      "    else if (fmul(zr, zr) + fmul(zi, zi) > 4 * scale()) n",
      "    else iter(fmul(zr, zr) - fmul(zi, zi) + cr,",
      "              2 * fmul(zr, zi) + ci,",
      "              cr, ci, n + 1, cap);",
      "}",
      "",
      "fn shade(n, cap) {",
      "  return if (n >= cap) \"@\"",
      "    else if (n > 6) \"#\"",
      "    else if (n > 4) \"+\"",
      "    else if (n > 2) \".\"",
      "    else \" \";",
      "}",
      "",
      "fn row(py, w, h, cap) {",
      "  let line = \"\";",
      "  let px = 0;",
      "  while (px < w) {",
      "    let cr = (px * 3500) / w - 2500;",
      "    let ci = (py * 2400) / h - 1200;",
      "    line = line + shade(iter(0, 0, cr, ci, 0, cap), cap);",
      "    px = px + 1;",
      "  }",
      "  return line;",
      "}",
      "",
      "fn draw() !io {",
      "  let py = 0;",
      "  while (py < 20) {",
      "    print(row(py, 60, 20, 12));",
      "    py = py + 1;",
      "  }",
      "  return 0;",
      "}"
    ].join("\n"),

    "fizzbuzz": [
      "// FizzBuzz with the rule set declared instead of buried in ifs.",
      "// Run `run` with an argument like 15.",
      "fn fizz(n)  { return \"Fizz\"; }",
      "fn buzz(n)  { return \"Buzz\"; }",
      "fn both(n)  { return \"FizzBuzz\"; }",
      "fn plain(n) { return str(n); }",
      "",
      "table rules {",
      "  \"3\"  -> fizz,",
      "  \"5\"  -> buzz,",
      "  \"15\" -> both,",
      "  \"1\"  -> plain,",
      "}",
      "",
      "fn key(n) {",
      "  return if (n % 15 == 0) \"15\"",
      "    else if (n % 5 == 0) \"5\"",
      "    else if (n % 3 == 0) \"3\"",
      "    else \"1\";",
      "}",
      "",
      "fn line(n) { return rules[key(n)](n); }",
      "",
      "fn run(upto) !io {",
      "  let i = 1;",
      "  while (i <= upto) {",
      "    print(line(i));",
      "    i = i + 1;",
      "  }",
      "  return upto;",
      "}",
      "",
      "fn test_fizz() { return line(9) == \"Fizz\"; }",
      "fn test_both() { return line(30) == \"FizzBuzz\"; }"
    ].join("\n"),

    "combat": [
      "// A combat resolver. The move set is declared, so \"what can happen",
      "// here?\" is answerable — and swapping two moves moves the table's hash.",
      "fn slash(atk, def)  { return { hit: atk * 2 - def, note: \"slash\" }; }",
      "fn pierce(atk, def) { return { hit: atk * 3 - def * 2, note: \"pierce\" }; }",
      "fn bash(atk, def)   { return { hit: atk + 4 - def, note: \"bash\" }; }",
      "fn heal(atk, def)   { return { hit: 0 - atk, note: \"heal\" }; }",
      "",
      "table moves {",
      "  \"slash\"  -> slash,",
      "  \"pierce\" -> pierce,",
      "  \"bash\"   -> bash,",
      "  \"heal\"   -> heal,",
      "}",
      "",
      "fn resolve(move, atk, def) {",
      "  let r = moves[move](atk, def);",
      "  return r.note + \" for \" + str(if (r.hit < 0) 0 - r.hit else r.hit)",
      "    + (if (r.hit < 0) \" healed\" else \" damage\");",
      "}",
      "",
      "fn fight() !io {",
      "  print(resolve(\"slash\", 7, 3));",
      "  print(resolve(\"pierce\", 7, 3));",
      "  print(resolve(\"heal\", 5, 0));",
      "  return 0;",
      "}",
      "",
      "fn test_pierce() { return resolve(\"pierce\", 7, 3) == \"pierce for 15 damage\"; }",
      "fn test_heal()   { return resolve(\"heal\", 5, 0) == \"heal for 5 healed\"; }"
    ].join("\n"),

    "memo": [
      "// Press Run. Then delete the word `memo` below and Run again —",
      "// watch the timer above the output.",
      "//",
      "// The cache is keyed on this definition's hash plus its arguments, so",
      "// it cannot go stale: change anything fib computes, or anything fib",
      "// depends on, and the key moves with it.",
      "memo fn fib(n) {",
      "  return if (n < 2) n else fib(n - 1) + fib(n - 2);",
      "}",
      "",
      "fn show() !io {",
      "  print(\"fib(27) = \" + str(fib(27)));",
      "  return fib(27);",
      "}",
      "",
      "fn test_fib() { return fib(20) == 6765; }"
    ].join("\n"),

    "rot13": [
      "// ROT13 over a declared alphabet — string handling with no stdlib.",
      "fn alphabet() { return \"abcdefghijklmnopqrstuvwxyz\"; }",
      "",
      "fn index_of(hay, ch, i) {",
      "  return if (i >= len(hay)) 0 - 1",
      "    else if (hay[i] == ch) i",
      "    else index_of(hay, ch, i + 1);",
      "}",
      "",
      "fn rot_char(ch) {",
      "  let i = index_of(alphabet(), ch, 0);",
      "  return if (i < 0) ch else alphabet()[(i + 13) % 26];",
      "}",
      "",
      "fn rot13(s) {",
      "  let out = \"\";",
      "  let i = 0;",
      "  while (i < len(s)) {",
      "    out = out + rot_char(s[i]);",
      "    i = i + 1;",
      "  }",
      "  return out;",
      "}",
      "",
      "fn demo() !io {",
      "  print(rot13(\"attack at dawn\"));",
      "  print(rot13(rot13(\"attack at dawn\")));",
      "  return 0;",
      "}",
      "",
      "fn test_rot()   { return rot13(\"abc\") == \"nop\"; }",
      "fn test_twice() { return rot13(rot13(\"hello there\")) == \"hello there\"; }"
    ].join("\n"),

    "the silent rebind": [
      "// Press Check. Then rename shout to yell, and give a DIFFERENT",
      "// function the name shout. Every other language compiles that.",
      "fn shout(s) {",
      "  return s + \"!\";",
      "}",
      "",
      "fn banner(s) {",
      "  return shout(s) + shout(s);",
      "}",
      "",
      "fn test_banner() { return banner(\"hi\") == \"hi!hi!\"; }"
    ].join("\n")
  };

  // Entry point and arguments that actually work, per example.
  var PRESET = {
    "mandelbrot":            { entry: "play.rg#draw",    args: "" },
    "fizzbuzz":              { entry: "play.rg#run",     args: "15" },
    "combat":                { entry: "play.rg#resolve", args: "pierce 7 3" },
    "memo":                  { entry: "play.rg#show",    args: "" },
    "rot13":                 { entry: "play.rg#demo",    args: "" },
    "the silent rebind":     { entry: "play.rg#banner",  args: "hi" }
  };

  Object.keys(EXAMPLES).forEach(function (name) {
    var o = document.createElement("option");
    o.value = name;
    o.textContent = name;
    picker.appendChild(o);
  });

  picker.addEventListener("change", function () {
    var src = EXAMPLES[picker.value];
    if (!src) return;
    editor.value = src;
    var preset = PRESET[picker.value];
    if (preset) {
      entry.value = preset.entry;
      argsBox.value = preset.args;
      entryTouched = true;
    }
    syncGutter();
    hintArgs();
    write("", "Loaded example: " + picker.value + ". Press Check, then Run or Test.");
  });

  // ── editor chrome ────────────────────────────────────────────────────
  function syncGutter() {
    var n = editor.value.split("\n").length;
    var s = "";
    for (var i = 1; i <= n; i++) s += i + "\n";
    gutter.textContent = s;
    gutter.scrollTop = editor.scrollTop;
  }
  editor.addEventListener("input", function () { syncGutter(); guessEntry(); });
  editor.addEventListener("scroll", function () { gutter.scrollTop = editor.scrollTop; });

  // Tab inserts two spaces instead of leaving the editor.
  editor.addEventListener("keydown", function (e) {
    if (e.key === "Tab") {
      e.preventDefault();
      var s = editor.selectionStart, t = editor.selectionEnd;
      editor.value = editor.value.slice(0, s) + "  " + editor.value.slice(t);
      editor.selectionStart = editor.selectionEnd = s + 2;
      syncGutter();
    }
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      send("run");
    }
  });

  // Offer a sensible entry point: prefer main, else the last defined function.
  var entryTouched = false;
  entry.addEventListener("input", function () { entryTouched = true; hintArgs(); });

  // Show what the chosen definition actually takes, read off its `fn` line.
  function paramsFor(name) {
    var bare = String(name || "").split("#").pop();
    if (!bare) return null;
    var re = new RegExp("^\\s*(?:memo\\s+)?fn\\s+" + bare.replace(/[^\w]/g, "") + "\\s*\\(([^)]*)\\)", "m");
    var m = re.exec(editor.value);
    if (!m) return null;
    return m[1].split(",").map(function (s) { return s.trim(); }).filter(Boolean);
  }

  function hintArgs() {
    var ps = paramsFor(entry.value);
    if (ps === null) { argsBox.placeholder = ""; argsBox.title = ""; return; }
    argsBox.placeholder = ps.length ? ps.join(" ") : "takes none";
    argsBox.title = ps.length
      ? "This definition takes " + ps.length + ": " + ps.join(", ")
      : "This definition takes no arguments.";
  }
  function guessEntry() {
    if (entryTouched) return;
    var names = [], m, re = /^\s*(?:memo\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)/gm;
    while ((m = re.exec(editor.value))) {
      if (m[1].indexOf("test_") !== 0) names.push(m[1]);
    }
    var pick = names.indexOf("main") >= 0 ? "main" : names[names.length - 1];
    entry.value = pick ? "play.rg#" + pick : "";
    hintArgs();
  }

  // ── output ───────────────────────────────────────────────────────────
  function esc(s) {
    return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }
  function write(html, statusText) {
    out.innerHTML = html || '<span class="dim">No output.</span>';
    if (statusText !== undefined) status.textContent = statusText;
  }

  function render(res, ms) {
    var parts = [];
    (res.diagnostics || []).forEach(function (d) {
      var cls = d.severity === "error" ? "er" : d.severity === "warning" ? "wr" : "in";
      var where = d.span ? " " + d.file + ":" + d.span.line + ":" + d.span.col : "";
      parts.push('<span class="' + cls + '">' + esc(d.severity + "[" + d.code + "]") + "</span>" +
                 esc(where + ": " + d.message));
    });
    if (res.output) parts.push(esc(res.output.replace(/\n$/, "")));
    if (res.result) {
      parts.push('<span class="' + (res.ok ? "ok" : "er") + '">' + esc(res.result) + "</span>");
    }
    write(parts.join("\n"), res.ok ? "Done in " + ms + " ms" : "Finished with errors");
  }

  // ── worker lifecycle ─────────────────────────────────────────────────
  function spawn() {
    worker = new Worker("play-worker.js");
    booted = false;
    worker.onmessage = function (e) {
      var m = e.data;
      if (m.type === "ready") {
        booted = true;
        setBusy(false);
        status.textContent = "Ready.";
        return;
      }
      if (m.type === "boot-error") {
        setBusy(true);
        write('<span class="er">' + esc(m.message) + "</span>\n\n" +
              esc("The playground needs to be served over http. Opening the page " +
                  "directly from disk blocks loading the WebAssembly module."),
              "Could not start");
        return;
      }
      if (m.type === "result" && pending !== null && m.id === pending) {
        clearTimeout(timer);
        pending = null;
        setBusy(false);
        render(m.result, m.ms);
      }
    };
  }

  function setBusy(busy) {
    buttons.forEach(function (b) { b.disabled = busy; });
  }

  function send(action) {
    if (!booted || pending !== null) return;
    var request = {
      source: editor.value,
      action: action,
      entry: entry.value.trim(),
      args: argsBox.value.trim() ? argsBox.value.trim().split(/\s+/) : []
    };
    if (action === "run" && !request.entry) {
      write('<span class="er">Set an entry point, for example play.rg#main.</span>', "Nothing to run");
      return;
    }
    pending = ++seq;
    setBusy(true);
    status.textContent = action === "test" ? "Running tests…" : "Running…";
    worker.postMessage({ id: pending, request: request });

    timer = setTimeout(function () {
      // The program did not come back. Kill it and start a fresh worker;
      // this is why execution lives off the main thread.
      worker.terminate();
      pending = null;
      spawn();
      setBusy(true);
      write('<span class="er">Stopped after ' + (TIMEOUT_MS / 1000) + " seconds.</span>\n" +
            esc("An unbounded `while` loop will do this. The worker was restarted, " +
                "so the test cache is now empty."),
            "Timed out");
    }, TIMEOUT_MS);
  }

  buttons.forEach(function (b) {
    b.addEventListener("click", function () { send(b.getAttribute("data-action")); });
  });

  // ── start ────────────────────────────────────────────────────────────
  editor.value = EXAMPLES["mandelbrot"];
  picker.value = "mandelbrot";
  entry.value = PRESET["mandelbrot"].entry;
  argsBox.value = PRESET["mandelbrot"].args;
  entryTouched = true;
  syncGutter();
  hintArgs();
  setBusy(true);
  status.textContent = "Loading the compiler…";
  spawn();
})();
