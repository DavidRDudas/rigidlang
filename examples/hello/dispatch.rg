// dispatch.rg — quarantined dynamism: first-class refs, dyn calls

fn double(x) {
  return x * 2;
}

fn triple(x) {
  return x * 3;
}

fn quadruple(x) {
  return x * 4;
}

fn pick(k) {
  return if (k == 2) &double else &triple;
}

fn apply(k, x) {
  let f = pick(k);
  return dyn { double, triple }(f, x);
}

fn escape(x) {
  // quadruple is NOT in the declared set: runtime quarantine violation
  return dyn { double, triple }(&quadruple, x);
}
