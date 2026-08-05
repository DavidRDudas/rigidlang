// util.rg — leaf definitions

fn clamp(x, lo, hi) {
  return if (x < lo) lo else if (x > hi) hi else x;
}

fn scale(x, k) {
  return x * k;
}
