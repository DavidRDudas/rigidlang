// Lists.
//
// Higher-order functions take a closure and invoke it through a declared
// `dyn` set, so the graph stays exact: `rdeps` on any lambda passed to `map`
// reaches this module and everything downstream of it. The set names the
// lifted lambdas the compiler created for the callers in this workspace —
// `rigid fix --apply` writes it, and a target outside it is refused by hash
// at runtime rather than silently invoked.

fn length(xs) {
  return len(xs);
}

fn is_empty(xs) {
  return len(xs) == 0;
}

fn first(xs, fallback) {
  if (len(xs) == 0) {
    return fallback;
  }
  return xs[0];
}

fn last(xs, fallback) {
  if (len(xs) == 0) {
    return fallback;
  }
  return xs[len(xs) - 1];
}

fn reverse(xs) {
  let out = [];
  let i = len(xs) - 1;
  while (i >= 0) {
    out = push(out, xs[i]);
    i = i - 1;
  }
  return out;
}

fn slice(xs, start, count) {
  let out = [];
  let i = start;
  let stop = start + count;
  if (stop > len(xs)) {
    stop = len(xs);
  }
  while (i < stop) {
    if (i >= 0) {
      out = push(out, xs[i]);
    }
    i = i + 1;
  }
  return out;
}

fn take(xs, n) {
  return slice(xs, 0, n);
}

fn drop(xs, n) {
  return slice(xs, n, len(xs));
}

fn index_of(xs, v) {
  let i = 0;
  for (x in xs) {
    if (x == v) {
      return i;
    }
    i = i + 1;
  }
  return -1;
}

fn contains(xs, v) {
  return index_of(xs, v) >= 0;
}

fn unique(xs) {
  let out = [];
  for (x in xs) {
    if (index_of(out, x) < 0) {
      out = push(out, x);
    }
  }
  return out;
}

fn concat(a, b) {
  return a + b;
}

fn flatten(xss) {
  let out = [];
  for (xs in xss) {
    out = out + xs;
  }
  return out;
}

fn sum(xs) {
  let acc = 0;
  for (x in xs) {
    acc = acc + x;
  }
  return acc;
}

fn min_of(xs, fallback) {
  if (len(xs) == 0) {
    return fallback;
  }
  let best = xs[0];
  for (x in xs) {
    if (x < best) {
      best = x;
    }
  }
  return best;
}

fn max_of(xs, fallback) {
  if (len(xs) == 0) {
    return fallback;
  }
  let best = xs[0];
  for (x in xs) {
    if (x > best) {
      best = x;
    }
  }
  return best;
}

// Insertion sort: the lists this language builds are small, and a stable,
// obviously-correct sort is worth more here than an asymptotically better one.
fn sort(xs) {
  let out = [];
  for (x in xs) {
    let placed = [];
    let done = 0;
    for (y in out) {
      if (done == 0) {
        if (x < y) {
          placed = push(placed, x);
          done = 1;
        }
      }
      placed = push(placed, y);
    }
    if (done == 0) {
      placed = push(placed, x);
    }
    out = placed;
  }
  return out;
}

fn test_reverse() { return reverse([1, 2, 3]) == [3, 2, 1]; }
fn test_slice() { return slice([1, 2, 3, 4, 5], 1, 3) == [2, 3, 4]; }
fn test_index_of() { return index_of(["a", "b"], "b") == 1; }
fn test_unique() { return unique([1, 1, 2, 1, 3]) == [1, 2, 3]; }
fn test_sum() { return sum([1, 2, 3, 4]) == 10; }
fn test_sort() { return sort([3, 1, 2]) == [1, 2, 3]; }
fn test_sort_stable_empty() { return sort([]) == []; }
fn test_min() { return min_of([3, 1, 2], 0) == 1; }
fn test_max() { return max_of([3, 1, 2], 0) == 3; }
fn test_flatten() { return flatten([[1], [2, 3]]) == [1, 2, 3]; }

property fn sort_preserves_length(xs: list) {
  return len(sort(xs)) == len(xs);
}

property fn sort_is_idempotent(xs: list) {
  return sort(sort(xs)) == sort(xs);
}

property fn reverse_is_its_own_inverse(xs: list) {
  return reverse(reverse(xs)) == xs;
}

property fn unique_is_idempotent(xs: list) {
  return unique(unique(xs)) == unique(xs);
}

property fn sum_ignores_order(xs: list) {
  return sum(xs) == sum(reverse(xs));
}

property fn take_and_drop_rejoin(xs: list) {
  return take(xs, 2) + drop(xs, 2) == xs;
}
