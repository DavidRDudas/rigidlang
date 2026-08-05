// logic.rg — mutual recursion (legal as of v1: SCC hashing)

fn is_even(n) {
  return if (n == 0) 1 else is_odd(n - 1);
}

fn is_odd(n) {
  return if (n == 0) 0 else is_even(n - 1);
}
