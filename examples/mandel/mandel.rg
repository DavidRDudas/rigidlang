// mandel.rg — the Mandelbrot set in a language with no floating point.
// Fixed-point arithmetic at scale 10000; a pure, tested, recursive
// escape-time core; palettes dispatched through a closed table.
//
//   rigid run "mandel.rg#main" classic
//   rigid run "mandel.rg#main" blocks

fn scale() { return 10000; }

fn fx_mul(a, b) {
  return (a * b) / scale();
}

// escape-time iteration: recursion is the loop, and it is pure
fn iterate(zr, zi, cr, ci, n, cap) {
  let zr2 = fx_mul(zr, zr);
  let zi2 = fx_mul(zi, zi);
  return if (n >= cap) cap
    else if (zr2 + zi2 > 4 * scale()) n
    else iterate(zr2 - zi2 + cr, 2 * fx_mul(zr, zi) + ci, cr, ci, n + 1, cap);
}

fn classic(n, cap) {
  let ramp = " .:-=+*#%@";
  return ramp[(n * (len(ramp) - 1)) / cap];
}

fn blocks(n, cap) {
  return if (n >= cap) "#"
    else if (n > cap / 2) "+"
    else if (n > cap / 4) "."
    else " ";
}

table palettes {
  "classic" -> classic,
  "blocks"  -> blocks,
}

fn row(py, w, h, cap, pal) {
  let xmin = 0 - (25 * scale()) / 10;
  let xstep = ((35 * scale()) / 10) / w;
  let ymin = 0 - (125 * scale()) / 100;
  let ystep = ((25 * scale()) / 10) / h;
  let ci = ymin + py * ystep;
  let line = "";
  let px = 0;
  while (px < w) {
    let it = iterate(0, 0, xmin + px * xstep, ci, 0, cap);
    line = line + palettes[pal](it, cap);
    px = px + 1;
  }
  return line;
}

fn main(pal) !io {
  let h = 24;
  let py = 0;
  while (py < h) {
    print(row(py, 76, h, 28, pal));
    py = py + 1;
  }
  return h;
}

fn test_origin_is_inside() {
  return iterate(0, 0, 0, 0, 0, 28) == 28;
}
fn test_far_point_escapes() {
  return iterate(0, 0, 2 * scale(), 2 * scale(), 0, 28) < 3;
}
fn test_palette_ends() {
  return if (classic(28, 28) == "@") classic(0, 28) == " " else 0;
}
