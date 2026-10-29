// Arithmetic that the core language leaves out.

// The one integer with no positive counterpart: -(-2^63) is -2^63 again, so
// there is no honest answer and `abs` refuses rather than returning a
// negative number. Found by `property fn abs_is_never_negative`, which is
// what those are for.
fn min_int() {
  return 0 - 9223372036854775807 - 1;
}

fn abs(n) {
  if (n == min_int()) {
    return fail({ error: "abs", value: n });
  }
  if (n < 0) {
    return -n;
  }
  return n;
}

fn min(a, b) {
  if (a < b) {
    return a;
  }
  return b;
}

fn max(a, b) {
  if (a > b) {
    return a;
  }
  return b;
}

fn clamp(n, lo, hi) {
  if (n < lo) {
    return lo;
  }
  if (n > hi) {
    return hi;
  }
  return n;
}

fn sign(n) {
  if (n < 0) {
    return -1;
  }
  if (n > 0) {
    return 1;
  }
  return 0;
}

fn pow(base, exp) {
  let acc = 1;
  let i = 0;
  while (i < exp) {
    acc = acc * base;
    i = i + 1;
  }
  return acc;
}

fn gcd(a, b) {
  let x = abs(a);
  let y = abs(b);
  while (y != 0) {
    let t = x % y;
    x = y;
    y = t;
  }
  return x;
}

fn lcm(a, b) {
  if (a == 0) {
    return 0;
  }
  if (b == 0) {
    return 0;
  }
  return abs(a * b) / gcd(a, b);
}

// Integer square root by Newton's method: exact, and no float required.
// The first step is x / 2 + x % 2 rather than (x + 1) / 2, which overflows
// at the largest integer. (It used to wrap there, and isqrt returned a
// negative number; checked arithmetic is what caught it.) After that,
// x + n / x <= 2x stays in range, because x never exceeds 2^62.
fn isqrt(n) {
  if (n < 2) {
    return max(n, 0);
  }
  let x = n;
  let y = x / 2 + x % 2;
  while (y < x) {
    x = y;
    y = (x + n / x) / 2;
  }
  return x;
}

fn floor_div(a, b) {
  let q = a / b;
  if (a % b != 0) {
    if (sign(a) != sign(b)) {
      return q - 1;
    }
  }
  return q;
}

fn round_to(x, places) {
  let scale = pow(10, places);
  let scaled = x * float(scale);
  let shifted = scaled + 0.5;
  if (x < 0.0) {
    shifted = scaled - 0.5;
  }
  return float(int(shifted)) / float(scale);
}

fn test_abs() { return abs(-5) == 5; }
fn test_clamp() { return clamp(12, 0, 10) == 10; }
fn test_pow() { return pow(2, 10) == 1024; }
fn test_gcd() { return gcd(12, 18) == 6; }
fn test_lcm() { return lcm(4, 6) == 12; }
fn test_isqrt() { return isqrt(17) == 4; }
fn test_isqrt_exact() { return isqrt(144) == 12; }
fn test_isqrt_max() { return isqrt(9223372036854775807) == 3037000499; }
fn test_floor_div() { return floor_div(-7, 2) == -4; }
fn test_round() { return round_to(3.14159, 2) == 3.14; }
fn test_sign() { return sign(-3) == -1; }
fn test_abs_min_int_fails() { return try(abs(min_int())).ok == 0; }

// Properties: claims over every input, not the handful anyone thought of.
property fn abs_is_never_negative(n: int) {
  let r = try(abs(n));
  return if (r.ok == 0) 1 else r.value >= 0;
}

property fn abs_is_idempotent(n: int) {
  let a = try(abs(n));
  return if (a.ok == 0) 1 else abs(a.value) == a.value;
}

property fn clamp_lands_in_range(n: int) {
  let c = clamp(n, -5, 5);
  return if (c < -5) 0 else if (c > 5) 0 else 1;
}

property fn isqrt_is_the_floor_of_the_root(n: int) {
  let m = try(abs(n));
  return if (m.ok == 0) 1 else isqrt(m.value) * isqrt(m.value) <= m.value;
}

property fn gcd_is_never_negative(a: int, b: int) {
  let g = try(gcd(a, b));
  return if (g.ok == 0) 1 else g.value >= 0;
}

property fn sign_is_minus_one_zero_or_one(n: int) {
  let s = sign(n);
  return if (s == -1) 1 else if (s == 0) 1 else if (s == 1) 1 else 0;
}
