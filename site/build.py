#!/usr/bin/env python3
"""Render the repo's Markdown docs into site pages.

The .md files stay the single source of truth; run this after editing them:

    python3 site/build.py

Handles exactly the Markdown these documents use — headings, paragraphs,
fenced and indented code, pipe tables, ordered and unordered lists, and the
inline forms (**bold**, *italic*, `code`, [text](url)). Standard library only,
so it needs nothing installed.
"""

import html
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "site"

# (source, output, nav title, one-line description)
DOCS = [
    ("docs/PAPER.md", "paper.html", "Paper",
     "The design of rigid and its evaluation."),
    ("docs/STUDY.md", "study.html", "Study",
     "How wide real dispatch tables are, measured over five Python projects."),
    ("docs/BENCHMARK.md", "benchmark.html", "Benchmark",
     "Breakage detection measured against TypeScript's compiler, scenario by scenario."),
    ("docs/EXPERIMENT.md", "experiment.html", "Experiment",
     "A pilot: does a machine-readable spec help a model answer dependency questions?"),
    ("docs/QUARANTINE.md", "quarantine.html", "Quarantine",
     "Why dynamism is declared rather than banned: dyn, tables, and guarantees G1-G4."),
    ("docs/PREREGISTRATION.md", "prereg.html", "Pre-registration",
     "The corpus and method, registered before any data was collected."),
    ("docs/EVIDENCE.md", "evidence.html", "Evidence",
     "Two measurements on code this project did not write: capability drift "
     "across 3,533 npm upgrades, and renames across 22,583 commits."),
]

# One nav and one footer for every page. build.py owns both: the generated
# pages get them from the template, and the hand-written pages are rewritten
# in place by sync_static_pages(), so the two can never drift apart.
# Every page, grouped for the header's picker. One list: the header shows all
# of them, so nothing needs a second home in the footer.
PAGE_GROUPS = [
    ("Language", [
        ("index.html", "Home"),
        ("learn.html", "Learn"),
        ("play.html", "Playground"),
        ("spec.html", "Specification"),
        ("tools.html", "Tools"),
    ]),
    ("Design", [
        ("paper.html", "Paper"),
        ("quarantine.html", "Quarantine"),
    ]),
    ("Evidence", [
        ("evidence.html", "Evidence"),
        ("benchmark.html", "Benchmark"),
        ("experiment.html", "Experiment"),
        ("study.html", "Study"),
        ("prereg.html", "Pre-registration"),
    ]),
]

AUTHOR_URL = "https://dudas.ai"
REPO_URL = "https://github.com/DavidRDudas/rigidlang"

# Pages written by hand; build.py only refreshes their nav and footer.
STATIC_PAGES = ["index.html", "learn.html", "spec.html", "play.html", "tools.html"]

ASSET_V = "9"

# The site never shows bare .md filenames: a mention of a document that has a
# page becomes a link to it, phrased the way you would say it out loud.
DOC_LINKS = {
    "docs/PAPER.md": ("paper.html", "the paper"),
    "docs/STUDY.md": ("study.html", "the study"),
    "docs/QUARANTINE.md": ("quarantine.html", "the quarantine design note"),
    "docs/PREREGISTRATION.md": ("prereg.html", "the pre-registration"),
    "docs/SPEC.md": ("spec.html", "the specification"),
    "docs/TUTORIAL.md": ("learn.html", "the tutorial"),
    "docs/EVIDENCE.md": ("evidence.html", "the evidence"),
}


def link_docs(body):
    """Replace NAME.md mentions in rendered prose with links to their pages.

    Both the bare name and the docs/-prefixed one, since the sources moved
    into docs/ but prose still says "SPEC.md" the way a person would.
    """
    for name, (href, label) in DOC_LINKS.items():
        bare = name.split("/")[-1]
        for form in (name, bare):
            body = body.replace(form, '<a href="%s">%s</a>' % (href, label))
    return body


def slug(text):
    s = re.sub(r"<[^>]+>", "", text).lower()
    s = re.sub(r"[^a-z0-9]+", "-", s).strip("-")
    return s or "section"


def inline(text):
    """Escape, then apply inline Markdown. Code spans are protected first."""
    spans = []

    def stash(m):
        spans.append(m.group(1))
        return "\x00%d\x00" % (len(spans) - 1)

    text = re.sub(r"`([^`]+)`", stash, text)
    text = html.escape(text, quote=False)
    text = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", r'<a href="\2">\1</a>', text)
    text = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", text)
    text = re.sub(r"(?<![*\w])\*([^*]+)\*(?!\*)", r"<em>\1</em>", text)
    text = re.sub(r"(?<!\w)_([^_]+)_(?!\w)", r"<em>\1</em>", text)
    return re.sub(
        r"\x00(\d+)\x00",
        lambda m: "<code>%s</code>" % html.escape(spans[int(m.group(1))], quote=False),
        text,
    )


def render_table(rows):
    """rows: list of raw '| a | b |' lines, second one being the separator."""
    def cells(line):
        return [c.strip() for c in line.strip().strip("|").split("|")]

    head, body = cells(rows[0]), [cells(r) for r in rows[2:]]
    out = ['<div class="tbl-wrap"><table class="cmp"><thead><tr>']
    out += ["<th>%s</th>" % inline(c) for c in head]
    out.append("</tr></thead><tbody>")
    for row in body:
        out.append("<tr>" + "".join("<td>%s</td>" % inline(c) for c in row) + "</tr>")
    out.append("</tbody></table></div>")
    return "".join(out)


def convert(md):
    """Markdown -> (html_body, page_title, [(id, text), ...] for the TOC)."""
    lines = md.split("\n")
    out, toc, title = [], [], None
    para, list_items, list_kind, table, code = [], [], None, [], None
    i = 0

    def flush_para():
        if para:
            out.append("<p>%s</p>" % inline(" ".join(para)))
            para.clear()

    def flush_list():
        nonlocal list_kind
        if list_items:
            tag = "ol" if list_kind == "ol" else "ul"
            out.append("<%s class='tight'>%s</%s>"
                       % (tag, "".join("<li>%s</li>" % inline(x) for x in list_items), tag))
            list_items.clear()
            list_kind = None

    def flush_table():
        if table:
            out.append(render_table(table))
            table.clear()

    def flush_all():
        flush_para(); flush_list(); flush_table()

    while i < len(lines):
        line = lines[i]

        # fenced code
        if line.startswith("```"):
            flush_all()
            i += 1
            buf = []
            while i < len(lines) and not lines[i].startswith("```"):
                buf.append(lines[i])
                i += 1
            out.append('<pre class="code"><code>%s</code></pre>'
                       % html.escape("\n".join(buf), quote=False))
            i += 1
            continue

        # indented code (4 spaces), only when not continuing a list
        if line.startswith("    ") and line.strip() and not list_items:
            flush_all()
            buf = []
            while i < len(lines) and (lines[i].startswith("    ") or not lines[i].strip()):
                if not lines[i].strip() and not (
                    i + 1 < len(lines) and lines[i + 1].startswith("    ")
                ):
                    break
                buf.append(lines[i][4:])
                i += 1
            out.append('<pre class="code"><code>%s</code></pre>'
                       % html.escape("\n".join(buf).rstrip(), quote=False))
            continue

        # table
        if line.startswith("|"):
            flush_para(); flush_list()
            table.append(line)
            i += 1
            continue
        flush_table()

        # headings
        m = re.match(r"^(#{1,4})\s+(.*)$", line)
        if m:
            flush_all()
            level, text = len(m.group(1)), m.group(2).strip()
            if level == 1 and title is None:
                title = re.sub(r"<[^>]+>", "", inline(text))
                out.append("<h1>%s</h1>" % inline(text))
            else:
                # Treat a second h1 as an h2 so the page keeps one document title.
                lvl = 2 if level == 1 else level
                sid = slug(text)
                if lvl == 2:
                    toc.append((sid, re.sub(r"<[^>]+>", "", inline(text))))
                out.append('<h%d id="%s">%s</h%d>' % (lvl, sid, inline(text), lvl))
            i += 1
            continue

        # lists
        m = re.match(r"^\s*[-*]\s+(.*)$", line)
        if m:
            flush_para()
            if list_kind == "ol":
                flush_list()
            list_kind = "ul"
            list_items.append(m.group(1))
            i += 1
            continue
        m = re.match(r"^\s*\d+\.\s+(.*)$", line)
        if m:
            flush_para()
            if list_kind == "ul":
                flush_list()
            list_kind = "ol"
            list_items.append(m.group(1))
            i += 1
            continue

        # continuation of a list item
        if list_items and line.strip() and line.startswith(("  ", "\t")):
            list_items[-1] += " " + line.strip()
            i += 1
            continue

        if not line.strip():
            flush_all()
            i += 1
            continue

        flush_list()
        para.append(line.strip())
        i += 1

    flush_all()
    return "\n".join(out), title or "Document", toc


ICON = ("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'%3E"
        "%3Cg stroke='%232F4BD8' stroke-width='2.4' fill='none'%3E%3Cpath d='M8 24 L16 8 L24 24'/%3E"
        "%3C/g%3E%3Cg fill='%232F4BD8'%3E%3Ccircle cx='8' cy='24' r='3.4'/%3E"
        "%3Ccircle cx='16' cy='8' r='3.4'/%3E%3Ccircle cx='24' cy='24' r='3.4'/%3E%3C/g%3E%3C/svg%3E")

GH_PATH = ("M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49"
           "-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 "
           "1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59."
           "82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27s1.36.09 2 .27c1.53-1.04 "
           "2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54."
           "73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0016 8c0-4.42-3.58-8-8-8z")

NAV_TEMPLATE = """<nav class="nav">
  <div class="wrap inner">
    <a class="brand" href="index.html">
      <svg viewBox="0 0 32 32" aria-hidden="true"><g stroke="var(--accent)" stroke-width="2.4" fill="none"><path d="M8 24 L16 8 L24 24"/></g><g fill="var(--accent)"><circle cx="8" cy="24" r="3.4"/><circle cx="16" cy="8" r="3.4"/><circle cx="24" cy="24" r="3.4"/></g></svg>
      <b>rigid</b><span class="ver">v1.8</span>
    </a>
    <select class="navpick" id="navpick" aria-label="Go to page">
%(options)s
    </select>
    <a class="lnk gh" href="%(repo)s" aria-label="GitHub">
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="%(gh)s"/></svg>
    </a>
    <button class="tbtn" id="theme" aria-label="Toggle dark mode">
      <svg class="i-moon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M21 12.8A9 9 0 1111.2 3a7 7 0 009.8 9.8z"/></svg>
      <svg class="i-sun" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>
    </button>
  </div>
</nav>"""

FOOTER_TEMPLATE = """<footer>
  <div class="wrap frow">
    <div>rigid — <a href="%(author)s">David Dudas</a></div>
  </div>
</footer>"""


def nav_html(current):
    """The shared header: brand, a picker over every page, GitHub, theme."""
    opts = []
    for group, pages in PAGE_GROUPS:
        opts.append('      <optgroup label="%s">' % group)
        for href, label in pages:
            sel = " selected" if href == current else ""
            opts.append('        <option value="%s"%s>%s</option>' % (href, sel, label))
        opts.append("      </optgroup>")
    return NAV_TEMPLATE % {
        "options": "\n".join(opts),
        "repo": REPO_URL,
        "gh": GH_PATH,
    }


def footer_html():
    return FOOTER_TEMPLATE % {"author": AUTHOR_URL}


def sync_static_pages():
    """Rewrite the nav and footer of the hand-written pages so all pages match.

    Anchored on the opening tag through the matching close; the templates above
    are the only definition of either block anywhere in the site.
    """
    done = []
    for name in STATIC_PAGES:
        path = OUT / name
        if not path.exists():
            continue
        original = path.read_text(encoding="utf-8")
        text = re.sub(r'<nav class="nav">.*?</nav>',
                      lambda _, n=name: nav_html(n), original, count=1, flags=re.S)
        text = re.sub(r"<footer>.*?</footer>",
                      lambda _: footer_html(), text, count=1, flags=re.S)
        if text != original:
            path.write_text(text, encoding="utf-8")
            done.append(name)
    return done


PAGE = """<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — rigid</title>
<meta name="description" content="{desc}">
<link rel="icon" href="{icon}">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:ital,wght@0,400;0,500;0,600;1,400&display=swap" rel="stylesheet">
<link rel="stylesheet" href="style.css?v={v}">
<script>
  try {{
    var t = localStorage.getItem("rigid-theme");
    if (t ? t === "dark" : matchMedia("(prefers-color-scheme: dark)").matches)
      document.documentElement.classList.add("dark");
  }} catch (e) {{}}
</script>
</head>
<body>

{nav}

<div class="wrap doc">
  <aside class="toc">
    <div class="t">{title}</div>
    <ol>
{toc}
    </ol>
  </aside>
  <main class="doc-body">
{body}
  </main>
</div>

{footer}

<script src="site.js?v={v}"></script>
</body>
</html>
"""


def build():
    written = []
    for src, outname, navtitle, desc in DOCS:
        path = ROOT / src
        if not path.exists():
            print("skip (missing): %s" % src, file=sys.stderr)
            continue
        body, title, toc = convert(path.read_text(encoding="utf-8"))
        body = link_docs(body)
        toc_html = "\n".join(
            '      <li><a href="#%s">%s</a></li>' % (sid, text) for sid, text in toc
        ) or '      <li class="grp" role="presentation">No sections</li>'
        page = PAGE.format(
            title=html.escape(navtitle), desc=html.escape(desc), icon=ICON, v=ASSET_V,
            nav=nav_html(outname), gh=GH_PATH, toc=toc_html, body=body,
            footer=footer_html(), src=src,
        )
        (OUT / outname).write_text(page, encoding="utf-8")
        written.append("%-18s -> site/%-18s %5d bytes, %2d sections"
                       % (src, outname, len(page), len(toc)))
    print("\n".join(written))
    synced = sync_static_pages()
    print("nav/footer synced: %s" % (", ".join(synced) if synced else "already current"))


if __name__ == "__main__":
    build()
