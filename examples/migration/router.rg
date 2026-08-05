// router.rg — the emit() call, migrated

import { lifecycle } from "./handlers.rg";
import { label } from "./core.rg";

fn route(rec) !io {
  return lifecycle[label(rec)](rec);
}

fn main() !io {
  route({ class: "task", action: "created", impact: 2, urgency: 3, age: 0, sla: 100 });
  route({ class: "task", action: "updated", impact: 3, urgency: 3, age: 10, sla: 100 });
  return route({ class: "sla", action: "breach", impact: 4, urgency: 4, age: 90, sla: 100 });
}
