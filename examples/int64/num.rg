// A JavaScript number is a float64, so it cannot hold every 64-bit integer.
// These are the cases where a backend that used one would quietly disagree
// with rigid's own engine.

fn big() {
  return 9007199254740993;
}

fn big_plus_one() {
  return 9007199254740993 + 1;
}

fn wide_product() {
  return 3037000499 * 3037000499;
}

fn test_successor_is_visible() {
  return big_plus_one() - big() == 1;
}

fn test_product_is_exact() {
  return wide_product() == 9223372030926249001;
}
