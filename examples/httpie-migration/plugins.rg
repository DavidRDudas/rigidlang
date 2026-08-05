// plugins.rg — faithful migration of httpie's formatter plugin dispatch
// (httpie/plugins/registry.py + httpie/output/processing.py).
// Original: a PluginManager registry populated at import time, iterated
// dynamically; the set of formatters is unknowable statically.
// Migrated: the same four built-in formatters, declared.

fn headers_formatter(s) { return s + "[headers]"; }
fn json_formatter(s)    { return s + "[json]"; }
fn xml_formatter(s)     { return s + "[xml]"; }
fn color_formatter(s)   { return s + "[color]"; }

table formatters {
  "headers" -> headers_formatter,
  "json"    -> json_formatter,
  "xml"     -> xml_formatter,
  "colors"  -> color_formatter,
}

// Formatting.__init__ + format_body: apply enabled plugins in order
fn apply_group(content, groups, i) {
  return if (i >= len(groups)) content
    else apply_group(formatters[groups[i]](content), groups, i + 1);
}

fn format_body(content, mime, groups) {
  return if (is_valid_mime(mime)) apply_group(content, groups, 0) else content;
}

fn is_valid_mime(mime) {
  return if (len(mime) < 3) 0 else 1;
}

// auth plugin dispatch: the other registry in the same file
fn basic_auth(u) { return "Basic " + u; }
fn digest_auth(u) { return "Digest " + u; }
fn bearer_auth(u) { return "Bearer " + u; }

table auth_plugins {
  "basic"  -> basic_auth,
  "digest" -> digest_auth,
  "bearer" -> bearer_auth,
}

fn authenticate(scheme, user) {
  return auth_plugins[scheme](user);
}

fn test_format() {
  return format_body("body", "application/json", ["json", "colors"])
    == "body[json][color]";
}
fn test_auth() { return authenticate("bearer", "tok") == "Bearer tok"; }
fn test_bad_mime() { return format_body("x", "", ["json"]) == "x"; }
