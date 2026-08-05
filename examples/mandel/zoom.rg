// zoom.rg — an animated Mandelbrot zoom, in a language with no floats,
// no sleep, and no graphics. Fixed-point at scale 1000000; each frame
// re-renders at a tighter window around a chosen target point; frames
// are separated by ANSI home-and-clear so the terminal animates in place.
//
//   rigid run "zoom.rg#main" seahorse 40
//   rigid run "zoom.rg#main" elephant 40
//   rigid run "zoom.rg#main" spike 60

fn scale() { return 1000000; }

fn fx_mul(a, b) {
  return (a * b) / scale();
}

// escape-time core: pure, recursive, tested
fn iterate(zr, zi, cr, ci, n, cap) {
  let zr2 = fx_mul(zr, zr);
  let zi2 = fx_mul(zi, zi);
  return if (n >= cap) cap
    else if (zr2 + zi2 > 4 * scale()) n
    else iterate(zr2 - zi2 + cr, 2 * fx_mul(zr, zi) + ci, cr, ci, n + 1, cap);
}

fn shade(n, cap) {
  let ramp = " .:-=+*#%@";
  return ramp[(n * (len(ramp) - 1)) / cap];
}

// zoom targets: classic points of interest, in fixed-point coordinates
fn seahorse(unused) { return { cr: 0 - 743000, ci: 131000 }; }
fn elephant(unused) { return { cr: 275000, ci: 6000 }; }
fn spike(unused)    { return { cr: 0 - 1749000, ci: 0 }; }

table targets {
  "seahorse" -> seahorse,
  "elephant" -> elephant,
  "spike"    -> spike,
}

// geometric zoom: halve the window every `every` frames, integer-only
fn span_at(frame) {
  return if (frame == 0) 3000000
    else (span_at(frame - 1) * 88) / 100;
}

fn depth_at(frame) {
  return 40 + frame * 6;
}

fn render_row(py, w, h, cx, cy, span, cap) {
  let xstep = (span * 2) / w;
  let ystep = span / h;
  let ci = cy - span / 2 + py * ystep;
  let line = "";
  let px = 0;
  while (px < w) {
    let cr = cx - span + px * xstep;
    line = line + shade(iterate(0, 0, cr, ci, 0, cap), cap);
    px = px + 1;
  }
  return line;
}

fn frame_text(w, h, cx, cy, span, cap) {
  let out = "\e[H\e[2J";
  let py = 0;
  while (py < h) {
    out = out + render_row(py, w, h, cx, cy, span, cap) + "\n";
    py = py + 1;
  }
  return out;
}

fn play(frame, frames, w, h, cx, cy) !io {
  let span = span_at(frame);
  print(frame_text(w, h, cx, cy, span, depth_at(frame)));
  return if (frame + 1 >= frames) frames
    else play(frame + 1, frames, w, h, cx, cy);
}

fn main(name, frames) !io {
  let n = try(int(frames));
  let count = if (n.ok) n.value else 30;
  let t = targets[name](0);
  return play(0, count, 78, 22, t.cr, t.ci);
}

fn test_span_shrinks() { return span_at(10) < span_at(0); }
fn test_depth_grows() { return depth_at(10) > depth_at(0); }
fn test_center_inside() { return iterate(0, 0, 0, 0, 0, 40) == 40; }
fn test_target_lookup() { return seahorse(0).ci == 131000; }
