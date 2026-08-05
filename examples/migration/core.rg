// core.rg — pure logic, fully singleton

fn priority(impact, urgency) {
  let p = impact * urgency;
  return if (p > 9) 9 else p;
}

fn breach_risk(age, sla) {
  return if (sla == 0) 0 else (age * 10) / sla;
}

fn label(rec) {
  return rec.class + "." + rec.action;
}

fn test_priority() { return priority(4, 4) == 9; }
fn test_risk() { return breach_risk(50, 100) == 5; }
fn test_label() {
  return label({ class: "task", action: "closed", age: 1 }) == "task.closed";
}
