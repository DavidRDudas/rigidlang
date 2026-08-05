// Integers and floats are two domains. A backend with one number type has to
// choose which one to be wrong about.

fn int_div() {
  return 7 / 2;
}

fn float_div() {
  return 7.0 / 2;
}

fn area(r) {
  return 3.14159 * r * r;
}

fn half(n) {
  return float(n) / 2.0;
}

fn shown() {
  return str(2.0);
}

fn cross_eq() {
  return 1 == 1.0;
}

fn test_int_division_truncates() {
  return int_div() == 3;
}

fn test_float_division_does_not() {
  return float_div() == 3.5;
}

fn test_two_point_zero_shows_its_point() {
  return shown() == "2.0";
}
