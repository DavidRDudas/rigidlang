// router.rg — data-driven dispatch: string keys, closed set, exact graph

table handlers {
  "created" -> on_created,
  "resolved" -> on_resolved,
}

fn on_created(p) !io {
  return print(p * 10);
}

fn on_resolved(p) !io {
  return print(p + 1);
}

fn route(ev, p) !io {
  return handlers[ev](p);
}
