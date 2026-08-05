// md.rg — the markdown engine (pure). Block kinds are classified by a
// function and rendered through a dispatch table keyed by that kind:
// the string stays a string, and the set of renderers is a closed,
// hashed node in the graph.
import { starts_with, drop_prefix, find_from, split_lines } from "./strings.rg";
import { escape, tag, link } from "./html.rg";

// ---- inline: `code`, **bold**, *italic*, [text](url) ----
fn inline(s) {
  let out = "";
  let i = 0;
  while (i < len(s)) {
    let c = s[i];
    let two = if (i + 1 < len(s)) c + s[i + 1] else c;
    let r =
      if (c == "`") inline_span(s, i + 1, "`", "code")
      else if (two == "**") inline_span2(s, i + 2, "b")
      else if (c == "*") inline_span(s, i + 1, "*", "i")
      else if (c == "[") inline_link(s, i)
      else { stop: i + 1, text: c };
    out = out + r.text;
    i = r.stop;
  }
  return out;
}

// single-char delimited span; falls back to the literal char if unclosed
fn inline_span(s, start, close, tagname) {
  let e = find_from(s, close, start);
  return if (e < 0) { stop: start, text: close }
    else { stop: e + 1, text: tag(tagname, substr(s, start, e - start)) };
}

// ** ... ** span
fn inline_span2(s, start, tagname) {
  let e = find2(s, start);
  return if (e < 0) { stop: start, text: "**" }
    else { stop: e + 2, text: tag(tagname, substr(s, start, e - start)) };
}

fn find2(s, i) {
  return if (i + 1 >= len(s)) 0 - 1
    else if (s[i] == "*") (if (s[i + 1] == "*") i else find2(s, i + 1))
    else find2(s, i + 1);
}

// [text](url)
fn inline_link(s, i) {
  let ct = find_from(s, "]", i + 1);
  let ok = if (ct < 0) 0 else (if (ct + 1 < len(s)) s[ct + 1] == "(" else 0);
  let cu = if (ok) find_from(s, ")", ct + 2) else 0 - 1;
  return if (cu < 0) { stop: i + 1, text: "[" }
    else {
      stop: cu + 1,
      text: link(substr(s, ct + 2, cu - ct - 2), substr(s, i + 1, ct - i - 1))
    };
}

// ---- blocks ----
fn classify(line) {
  return if (len(line) == 0) "blank"
    else if (starts_with(line, "### ")) "h3"
    else if (starts_with(line, "## ")) "h2"
    else if (starts_with(line, "# ")) "h1"
    else if (starts_with(line, "- ")) "li"
    else if (starts_with(line, "> ")) "quote"
    else if (starts_with(line, "```")) "fence"
    else "para";
}

fn r_h1(line) { return tag("h1", inline(escape(drop_prefix(line, 2)))); }
fn r_h2(line) { return tag("h2", inline(escape(drop_prefix(line, 3)))); }
fn r_h3(line) { return tag("h3", inline(escape(drop_prefix(line, 4)))); }
fn r_li(line) { return tag("li", inline(escape(drop_prefix(line, 2)))); }
fn r_quote(line) { return tag("blockquote", inline(escape(drop_prefix(line, 2)))); }
fn r_para(line) { return tag("p", inline(escape(line))); }
fn r_blank(line) { return ""; }
fn r_fence(line) { return ""; }

table renderers {
  "h1" -> r_h1,
  "h2" -> r_h2,
  "h3" -> r_h3,
  "li" -> r_li,
  "quote" -> r_quote,
  "para" -> r_para,
  "blank" -> r_blank,
  "fence" -> r_fence,
}

// full document render: fence + list state as integers, one pass
fn render(src) {
  let lines = split_lines(src);
  let out = "";
  let in_code = 0;
  let in_list = 0;
  let i = 0;
  while (i < len(lines)) {
    let line = lines[i];
    let kind = classify(line);
    let fence_toggle = kind == "fence";
    out = out + (if (fence_toggle) (if (in_code) "</code></pre>\n" else "<pre><code>")
      else if (in_code) escape(line) + "\n"
      else "");
    in_code = if (fence_toggle) 1 - in_code else in_code;
    let plain = if (fence_toggle) 1 else in_code;
    out = out + (if (plain) ""
      else if (kind == "li") (if (in_list) "" else "<ul>\n")
      else (if (in_list) "</ul>\n" else ""));
    in_list = if (plain) in_list else (if (kind == "li") 1 else 0);
    out = out + (if (plain) ""
      else if (kind == "blank") ""
      else renderers[kind](line) + "\n");
    i = i + 1;
  }
  return out + (if (in_list) "</ul>\n" else "") + (if (in_code) "</code></pre>\n" else "");
}

// first h1 becomes the page title; falls back to the given default
fn title_of(src, default) {
  let lines = split_lines(src);
  let i = 0;
  let found = "";
  while (i < len(lines)) {
    found = if (found == "") (if (classify(lines[i]) == "h1") drop_prefix(lines[i], 2) else "") else found;
    i = i + 1;
  }
  return if (found == "") default else found;
}

fn test_h1() { return render("# Hi") == "<h1>Hi</h1>\n"; }
fn test_bold() { return inline("a **b** c") == "a <b>b</b> c"; }
fn test_code_span() { return inline("x `y` z") == "x <code>y</code> z"; }
fn test_link_inline() { return inline("[A](a.html)") == "<a href=\"a.html\">A</a>"; }
fn test_unclosed_bold() { return inline("a **b") == "a **b"; }
fn test_escape_in_para() { return render("1 < 2") == "<p>1 &lt; 2</p>\n"; }
fn test_list_wraps() {
  return render("- a\n- b") == "<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n";
}
fn test_fence_literal() {
  return render("```\n**not bold**\n```") == "<pre><code>**not bold**\n</code></pre>\n";
}
fn test_quote() { return render("> wise") == "<blockquote>wise</blockquote>\n"; }
fn test_title() { return title_of("junk\n# Real Title\nmore", "d") == "Real Title"; }
fn test_title_default() { return title_of("no heading", "d") == "d"; }
