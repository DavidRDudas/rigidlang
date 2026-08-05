// Strings.
//
// rigid has no character codes, so case conversion is a table lookup over the
// alphabet rather than arithmetic on code points. That is slower and it is
// also honest: the language has no `ord`, so nothing here pretends otherwise.

fn lower_alphabet() {
  return "abcdefghijklmnopqrstuvwxyz";
}

fn upper_alphabet() {
  return "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
}

fn char_index(alphabet, c) {
  let i = 0;
  for (a in alphabet) {
    if (a == c) {
      return i;
    }
    i = i + 1;
  }
  return -1;
}

fn to_lower(s) {
  let out = "";
  for (c in s) {
    let i = char_index(upper_alphabet(), c);
    if (i < 0) {
      out = out + c;
    } else {
      out = out + substr(lower_alphabet(), i, 1);
    }
  }
  return out;
}

fn to_upper(s) {
  let out = "";
  for (c in s) {
    let i = char_index(lower_alphabet(), c);
    if (i < 0) {
      out = out + c;
    } else {
      out = out + substr(upper_alphabet(), i, 1);
    }
  }
  return out;
}

fn starts_with(s, prefix) {
  if (len(prefix) > len(s)) {
    return 0;
  }
  return substr(s, 0, len(prefix)) == prefix;
}

fn ends_with(s, suffix) {
  if (len(suffix) > len(s)) {
    return 0;
  }
  return substr(s, len(s) - len(suffix), len(suffix)) == suffix;
}

fn find(s, needle) {
  if (len(needle) == 0) {
    return 0;
  }
  let i = 0;
  while (i + len(needle) <= len(s)) {
    if (substr(s, i, len(needle)) == needle) {
      return i;
    }
    i = i + 1;
  }
  return -1;
}

fn contains(s, needle) {
  return find(s, needle) >= 0;
}

fn split(s, sep) {
  if (len(sep) == 0) {
    return chars(s);
  }
  let out = [];
  let cur = "";
  let i = 0;
  while (i < len(s)) {
    if (i + len(sep) <= len(s)) {
      if (substr(s, i, len(sep)) == sep) {
        out = push(out, cur);
        cur = "";
        i = i + len(sep);
      } else {
        cur = cur + substr(s, i, 1);
        i = i + 1;
      }
    } else {
      cur = cur + substr(s, i, 1);
      i = i + 1;
    }
  }
  return push(out, cur);
}

fn replace(s, needle, to) {
  if (len(needle) == 0) {
    return s;
  }
  return join(split(s, needle), to);
}

fn trim_start(s) {
  let i = 0;
  while (i < len(s)) {
    let c = substr(s, i, 1);
    if (c == " ") {
      i = i + 1;
    } else if (c == "\t") {
      i = i + 1;
    } else if (c == "\n") {
      i = i + 1;
    } else if (c == "\r") {
      i = i + 1;
    } else {
      return substr(s, i, len(s) - i);
    }
  }
  return "";
}

fn trim_end(s) {
  let n = len(s);
  while (n > 0) {
    let c = substr(s, n - 1, 1);
    if (c == " ") {
      n = n - 1;
    } else if (c == "\t") {
      n = n - 1;
    } else if (c == "\n") {
      n = n - 1;
    } else if (c == "\r") {
      n = n - 1;
    } else {
      return substr(s, 0, n);
    }
  }
  return "";
}

fn trim(s) {
  return trim_end(trim_start(s));
}

fn repeat(s, n) {
  let out = "";
  let i = 0;
  while (i < n) {
    out = out + s;
    i = i + 1;
  }
  return out;
}

fn pad_left(s, width, fill) {
  if (len(s) >= width) {
    return s;
  }
  return repeat(fill, width - len(s)) + s;
}

fn pad_right(s, width, fill) {
  if (len(s) >= width) {
    return s;
  }
  return s + repeat(fill, width - len(s));
}

fn lines(s) {
  return split(s, "\n");
}

fn test_lower() { return to_lower("AbC") == "abc"; }
fn test_upper() { return to_upper("aBc") == "ABC"; }
fn test_starts() { return starts_with("hello", "he") == 1; }
fn test_ends() { return ends_with("hello", "lo") == 1; }
fn test_find() { return find("hello", "ll") == 2; }
fn test_find_miss() { return find("hello", "zz") == -1; }
fn test_split() { return split("a,b,c", ",") == ["a", "b", "c"]; }
fn test_replace() { return replace("a-b-c", "-", "+") == "a+b+c"; }
fn test_trim() { return trim("  hi  ") == "hi"; }
fn test_pad() { return pad_left("7", 3, "0") == "007"; }
fn test_lines() { return len(lines("a\nb")) == 2; }

property fn trim_is_idempotent(s: str) {
  return trim(trim(s)) == trim(s);
}

property fn lower_is_idempotent(s: str) {
  return to_lower(to_lower(s)) == to_lower(s);
}

property fn split_then_join_roundtrips(s: str) {
  return join(split(s, ","), ",") == s;
}

property fn a_string_starts_with_itself(s: str) {
  return starts_with(s, s) == 1;
}

property fn repeat_scales_length(s: str) {
  return len(repeat(s, 3)) == len(s) * 3;
}
