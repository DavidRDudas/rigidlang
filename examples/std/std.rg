// std.rg — string/number stdlib and file IO through the effect door

fn shout_each(words, i, acc) {
  return if (i >= len(words)) acc
    else shout_each(words, i + 1, push(acc, words[i] + "!"));
}

fn slug(s) {
  return join(shout_each(chars(s), 0, []), "-");
}

fn parse_or(s, fallback) {
  let r = try(int(s));
  return if (r.ok) r.value else fallback;
}

fn report(n) {
  return "count=" + str(n) + " (" + substr("critical", 0, 4) + ")";
}

fn save_report(path, n) !io {
  write_file(path, report(n));
  return read_file(path);
}

fn test_int_parse() { return int("42") == 42; }
fn test_int_catch() { return parse_or("nope", -1) == -1; }
fn test_substr() { return substr("hello world", 6, 5) == "world"; }
fn test_chars_join() { return slug("abc") == "a!-b!-c!"; }
fn test_str() { return report(7) == "count=7 (crit)"; }
