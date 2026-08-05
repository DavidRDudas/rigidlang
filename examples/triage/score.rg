// score.rg — pure severity math

fn clampi(x, lo, hi) {
  return if (x < lo) lo else if (x > hi) hi else x;
}

fn severity(impact, urgency) {
  return clampi(impact * urgency, 1, 9);
}

fn is_p1(sev) {
  return sev >= 8;
}
