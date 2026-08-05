fn sum_to(n) {
  let total = 0;
  let i = 1;
  while (i <= n) {
    total = total + i;
    i = i + 1;
  }
  return total;
}

fn fib_iter(n) {
  let a = 0;
  let b = 1;
  while (n > 0) {
    let t = a + b;
    a = b;
    b = t;
    n = n - 1;
  }
  return a;
}

fn checked_div(a, b) {
  return if (b == 0) fail({ code: 400, msg: "division by zero", num: a }) else a / b;
}

fn safe_div(a, b) {
  let r = try(checked_div(a, b));
  return if (r.ok) r.value else -1;
}

fn test_sum() { return sum_to(100) == 5050; }
fn test_fib() { return fib_iter(20) == 6765; }
fn test_safe() { return safe_div(10, 0) == -1; }
fn test_payload() {
  let r = try(checked_div(7, 0));
  return if (r.ok) 0 else r.error.code == 400;
}
fn test_builtin_catch() {
  let r = try([1, 2][9]);
  return if (r.ok) 0 else 1;
}
fn test_loop_let() {
  let out = [];
  let i = 0;
  while (i < 3) {
    let sq = i * i;
    out = push(out, sq);
    i = i + 1;
  }
  return if (len(out) == 3) out[2] == 4 else 0;
}

fn main() !io {
  print(fib_iter(30));
  let r = try(checked_div(9, 0));
  print(r);
  return sum_to(10);
}
