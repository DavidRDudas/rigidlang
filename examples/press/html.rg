// html.rg — escaping and tag construction (pure)
import { split_lines } from "./strings.rg";

fn escape(s) {
  let out = "";
  let i = 0;
  while (i < len(s)) {
    let c = s[i];
    out = out + (if (c == "&") "&amp;"
      else if (c == "<") "&lt;"
      else if (c == ">") "&gt;"
      else c);
    i = i + 1;
  }
  return out;
}

fn tag(name, inner) {
  return "<" + name + ">" + inner + "</" + name + ">";
}

fn link(href, text) {
  return "<a href=\"" + href + "\">" + text + "</a>";
}

fn page(title, nav, body) {
  return "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n"
    + tag("title", title)
    + "\n<style>body{max-width:70ch;margin:3rem auto;font-family:Georgia,serif;line-height:1.6;padding:0 1rem}pre{background:#f4f2ed;padding:1rem;overflow-x:auto}code{font-family:monospace}blockquote{border-left:3px solid #ccc;margin-left:0;padding-left:1rem;color:#555}nav{font-family:monospace;font-size:.85rem;margin-bottom:2rem}</style>\n"
    + "</head>\n<body>\n" + tag("nav", nav) + "\n" + body + "\n</body>\n</html>\n";
}

fn test_escape() { return escape("a<b&c>d") == "a&lt;b&amp;c&gt;d"; }
fn test_escape_clean() { return escape("plain") == "plain"; }
fn test_tag() { return tag("h1", "hi") == "<h1>hi</h1>"; }
fn test_link() { return link("a.html", "A") == "<a href=\"a.html\">A</a>"; }
