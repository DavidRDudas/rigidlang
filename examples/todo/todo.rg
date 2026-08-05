// todo.rg — a persistent todo list, written in rigid.
//   rigid run "todo.rg#main" add "buy milk"
//   rigid run "todo.rg#main" list x
//   rigid run "todo.rg#main" done 0
// Storage: one item per line in todo.txt — "o|task" open, "x|task" done.

fn todo_path() {
  return "todo.txt";
}

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

fn toggle(item) {
  return "x" + substr(item, 1, len(item) - 1);
}

fn render(item, idx) {
  let mark = if (substr(item, 0, 1) == "x") "[x] " else "[ ] ";
  return str(idx) + " " + mark + substr(item, 2, len(item) - 2);
}

fn load() !io {
  let r = try(read_file(todo_path()));
  return if (r.ok) split_lines(r.value) else [];
}

fn save(items) !io {
  write_file(todo_path(), join(items, "\n"));
  return len(items);
}

fn do_add(task) !io {
  let items = load();
  save(push(items, "o|" + task));
  return print("added: " + task);
}

fn do_list(unused) !io {
  let items = load();
  let i = 0;
  while (i < len(items)) {
    print(render(items[i], i));
    i = i + 1;
  }
  return len(items);
}

fn do_done(arg) !io {
  let n = try(int(arg));
  return if (n.ok) mark_done(n.value) else print("usage: done <number>");
}

fn mark_done(idx) !io {
  let items = load();
  let out = [];
  let i = 0;
  while (i < len(items)) {
    out = push(out, if (i == idx) toggle(items[i]) else items[i]);
    i = i + 1;
  }
  save(out);
  return print("done: " + str(idx));
}

table commands {
  "add"  -> do_add,
  "list" -> do_list,
  "done" -> do_done,
}

fn main(cmd, arg) !io {
  return commands[cmd](arg);
}

fn test_split() {
  let xs = split_lines("a\nbb\nccc");
  return if (len(xs) == 3) xs[2] == "ccc" else 0;
}
fn test_toggle() { return toggle("o|ship it") == "x|ship it"; }
fn test_render() { return render("x|done thing", 4) == "4 [x] done thing"; }
