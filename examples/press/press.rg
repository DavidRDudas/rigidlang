// press.rg — the !io shell. A site is a manifest: one markdown filename
// per line. Inputs are declared, not discovered — the rigid way.
import { split_lines, ends_with } from "./strings.rg";
import { render, title_of } from "./md.rg";
import { page, link } from "./html.rg";

fn out_name(md) {
  return if (ends_with(md, ".md")) substr(md, 0, len(md) - 3) + ".html" else md + ".html";
}

fn nav_for(names, i) {
  return if (i >= len(names)) ""
    else link(out_name(names[i]), out_name(names[i]))
      + (if (i + 1 < len(names)) " · " else "")
      + nav_for(names, i + 1);
}

fn build_one(name, nav) !io {
  let src = try(read_file(name));
  return if (src.ok) write_page(name, nav, src.value) else print("skip (unreadable): " + name);
}

fn write_page(name, nav, src) !io {
  let body = render(src);
  let html = page(title_of(src, name), nav, body);
  write_file(out_name(name), html);
  return print("wrote " + out_name(name) + " (" + str(len(html)) + " bytes)");
}

fn main(manifest) !io {
  let m = try(read_file(manifest));
  return if (m.ok) build_all(split_lines(m.value)) else print("no manifest: " + manifest);
}

fn build_all(names) !io {
  let nav = nav_for(names, 0);
  let i = 0;
  while (i < len(names)) {
    build_one(names[i], nav);
    i = i + 1;
  }
  return len(names);
}

fn test_out_name() { return out_name("guide.md") == "guide.html"; }
fn test_nav() {
  return nav_for(["a.md", "b.md"], 0)
    == "<a href=\"a.html\">a.html</a> · <a href=\"b.html\">b.html</a>";
}
