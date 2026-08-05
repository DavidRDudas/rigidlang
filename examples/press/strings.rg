// strings.rg — shared string utilities (pure)

fn split_lines(s) {
  let out = [];
  let cur = "";
  let i = 0;
  while (i < len(s)) {
    let c = s[i];
    out = if (c == "\n") push(out, cur) else out;
    cur = if (c == "\n") "" else cur + c;
    i = i + 1;
  }
  return if (cur == "") out else push(out, cur);
}

fn starts_with(s, p) {
  return if (len(s) < len(p)) 0 else substr(s, 0, len(p)) == p;
}

fn drop_prefix(s, n) {
  return substr(s, n, len(s) - n);
}

fn ends_with(s, p) {
  return if (len(s) < len(p)) 0
    else substr(s, len(s) - len(p), len(p)) == p;
}

// index of ch at or after i, or -1
fn find_from(s, ch, i) {
  return if (i >= len(s)) 0 - 1
    else if (s[i] == ch) i
    else find_from(s, ch, i + 1);
}

fn test_split() { return len(split_lines("a\nb\nc")) == 3; }
fn test_starts() { return starts_with("## title", "## "); }
fn test_ends() { return ends_with("page.md", ".md"); }
fn test_find() { return find_from("ab)cd", ")", 0) == 2; }
fn test_find_missing() { return find_from("abcd", ")", 0) == 0 - 1; }
