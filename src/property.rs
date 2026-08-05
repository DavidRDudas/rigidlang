//! Properties: claims that must hold for every input, not for an example.
//!
//! A test is a sentence about one input you thought of. A property is a
//! sentence about all of them, and the machine looks for the counterexample:
//!
//! ```text
//! property fn sorting_preserves_length(xs) {
//!   return len(sort(xs)) == len(xs);
//! }
//! ```
//!
//! What this adds over the property testers that have existed since
//! QuickCheck in 1999 is scoping. A property is a definition with a content
//! hash, so the workspace knows exactly which properties a change puts back
//! in question — the same reason unchanged tests can be skipped — and a
//! property that passed is evidence about an exact version of the code
//! rather than about "the repo, some time on Tuesday".
//!
//! Generation is deterministic and seeded from the property's own hash. That
//! is not a detail: it means a property that passes will pass again given
//! the same code, so the result is cacheable, and a failure is reproducible
//! by anyone with the same source.
//!
//! Inputs come from the parameter domains already inferred for `E0212`. A
//! parameter used in arithmetic gets numbers; one indexed gets lists; one
//! concatenated gets strings. Where nothing was inferred, integers are used,
//! and the report says so rather than pretending.

use crate::domains::Domain;
use crate::eval::Value;

/// xorshift64*, seeded from a definition's hash. Small, deterministic, and
/// nobody's cryptography depends on it.
pub struct Gen {
    state: u64,
}

impl Gen {
    pub fn from_hash(h: &crate::canon::Hash) -> Gen {
        let mut s = 0u64;
        for (i, b) in h.iter().take(8).enumerate() {
            s |= (*b as u64) << (8 * i);
        }
        Gen {
            state: if s == 0 { 0x9E3779B97F4A7C15 } else { s },
        }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }

    /// A value in `d`, of roughly `size`. Size grows across a run so the
    /// first cases are the small ones a human would have tried first.
    pub fn value(&mut self, d: Option<Domain>, size: u32) -> Value {
        match d {
            Some(Domain::Str) => Value::Str(self.string(size)),
            Some(Domain::List) => {
                let n = self.below(size as u64 + 1) as usize;
                Value::List((0..n).map(|_| Value::Int(self.int(size))).collect())
            }
            Some(Domain::Float) => {
                let a = self.int(size) as f64;
                let b = (self.below(100) as f64) / 100.0;
                Value::Float(a + b)
            }
            // A record's shape is not inferable from how it is used, so
            // there is nothing honest to generate. The runner refuses the
            // property rather than inventing a shape.
            Some(Domain::Record) => Value::Record(Default::default()),
            _ => Value::Int(self.int(size)),
        }
    }

    fn int(&mut self, size: u32) -> i64 {
        // Edge cases first — 0, 1, -1 and the boundaries are where the bugs
        // are, so they are drawn far more often than chance would give.
        match self.below(10) {
            0 => 0,
            1 => 1,
            2 => -1,
            3 => i64::MAX,
            4 => i64::MIN,
            _ => {
                let m = (size as i64).max(1) * 4;
                let v = (self.below((2 * m) as u64) as i64) - m;
                v
            }
        }
    }

    fn string(&mut self, size: u32) -> String {
        const ALPHABET: &[char] = &[
            'a', 'b', 'c', 'z', 'A', 'Z', '0', '9', ' ', '\n', '"', '\\', 'é', '中', '\u{1F600}',
        ];
        let n = self.below(size as u64 + 1) as usize;
        (0..n)
            .map(|_| ALPHABET[self.below(ALPHABET.len() as u64) as usize])
            .collect()
    }
}

/// Simpler candidates for one value, most aggressive first.
///
/// Shrinking is what makes a failure legible: the generator that found the
/// bug usually found it with a 40-element list of six-digit numbers, and
/// what you want to be told is `[0, 0]`.
pub fn shrink(v: &Value) -> Vec<Value> {
    match v {
        Value::Int(0) => vec![],
        Value::Int(n) => {
            let mut out = vec![Value::Int(0)];
            if *n != 1 && *n != -1 {
                out.push(Value::Int(n / 2));
                out.push(Value::Int(if *n > 0 { n - 1 } else { n + 1 }));
            }
            out
        }
        Value::Float(f) if *f == 0.0 => vec![],
        Value::Float(f) => vec![Value::Float(0.0), Value::Float(f / 2.0)],
        Value::Str(s) if s.is_empty() => vec![],
        Value::Str(s) => {
            let cs: Vec<char> = s.chars().collect();
            let mut out = vec![Value::Str(String::new())];
            if cs.len() > 1 {
                out.push(Value::Str(cs[..cs.len() / 2].iter().collect()));
                out.push(Value::Str(cs[1..].iter().collect()));
            }
            out
        }
        Value::List(xs) if xs.is_empty() => vec![],
        Value::List(xs) => {
            let mut out = vec![Value::List(Vec::new())];
            if xs.len() > 1 {
                out.push(Value::List(xs[..xs.len() / 2].to_vec()));
                out.push(Value::List(xs[1..].to_vec()));
            }
            // then simplify the elements, leaving the length alone
            for i in 0..xs.len() {
                for s in shrink(&xs[i]) {
                    let mut c = xs.clone();
                    c[i] = s;
                    out.push(Value::List(c));
                }
            }
            out
        }
        _ => vec![],
    }
}

/// Simpler candidates for a whole argument vector: one position at a time.
pub fn shrink_args(args: &[Value]) -> Vec<Vec<Value>> {
    let mut out = Vec::new();
    for i in 0..args.len() {
        for s in shrink(&args[i]) {
            let mut c = args.to_vec();
            c[i] = s;
            out.push(c);
        }
    }
    out
}

pub fn show_args(ws: &crate::resolve::Workspace, args: &[Value]) -> String {
    args.iter()
        .map(|a| crate::eval::show(ws, a))
        .collect::<Vec<_>>()
        .join(", ")
}
