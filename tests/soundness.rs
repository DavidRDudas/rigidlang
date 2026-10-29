//! Soundness tests: the guarantees rigid makes, checked by machine.
//!
//! Three properties are tested over generated programs, and every bug that
//! once broke one of them has a regression test of its own below.
//!
//!   1. Same hash, same behaviour. Renaming every definition, parameter and
//!      local, reordering record fields, and reformatting must leave every
//!      hash *and* every result unchanged. Test caching, `memo`, `attest`,
//!      `dup` and `diff --require-no-semantic-change` all stand on this.
//!   2. One meaning per program. The VM and the JavaScript backend produce
//!      the same output, error messages included. (Skipped without node.)
//!   3. No crash. Whatever a program does, the compiler and the VM return a
//!      value or an error; they never panic or exhaust the stack.
//!
//! RIGID_FUZZ_CASES (default 300) and RIGID_FUZZ_SEED widen a run locally.

use rigid::canon::Hash;
use rigid::diag::{Diagnostic, Severity};
use rigid::resolve::{DefId, Workspace};
use rigid::{backend_js, eval, resolve};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

// ------------------------------------------------------------------ harness

/// Run on a thread with room for the deepest tree the parser admits; debug
/// builds use far more stack per level than release builds do.
fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(4 << 30)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

struct Checked {
    ws: Workspace,
    hashes: HashMap<DefId, Hash>,
    diags: Vec<Diagnostic>,
}

fn load(files: &[(String, String)]) -> Checked {
    let mut ws = resolve::load_workspace_from_sources(files).unwrap_or_else(|d| {
        panic!("parse failed: {:?}", d.iter().map(|x| &x.message).collect::<Vec<_>>())
    });
    let (diags, hashes) = rigid::check_workspace(&mut ws);
    Checked { ws, hashes, diags }
}

fn src(files: &[(&str, &str)]) -> Vec<(String, String)> {
    files.iter().map(|(p, s)| (p.to_string(), s.to_string())).collect()
}

fn errors(c: &Checked) -> Vec<String> {
    c.diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{} {}: {}", d.code, d.file, d.message))
        .collect()
}

fn def(c: &Checked, module: &str, name: &str) -> DefId {
    *c.ws
        .def_index
        .get(&(module.to_string(), name.to_string()))
        .unwrap_or_else(|| panic!("no {}#{}", module, name))
}

fn hash(c: &Checked, module: &str, name: &str) -> Hash {
    c.hashes[&def(c, module, name)]
}

/// What `rigid run` would print for a call: the value, or the error.
fn run(c: &Checked, module: &str, name: &str, args: Vec<eval::Value>) -> String {
    let d = def(c, module, name);
    let eng = eval::Engine::new(&c.ws);
    match eng.call(d, args) {
        Ok(v) => eval::show(&c.ws, &v),
        Err(e) => format!("error: {}", eval::show(&c.ws, &e)),
    }
}

/// One-module program, checked clean, ready to run.
fn program(source: &str) -> Checked {
    let c = load(&src(&[("p.rg", source)]));
    assert!(errors(&c).is_empty(), "{:?}", errors(&c));
    c
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rigid-soundness-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rigid_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rigid")
}

fn node_available() -> bool {
    Command::new("node").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

// ------------------------------------------------------- regression tests

/// Record fields hash in name order. They once evaluated in source order,
/// so two literals with one hash could raise different errors — and a test
/// cache, a review attestation and `diff` all vouched for the swap.
#[test]
fn record_field_order_cannot_split_a_hash() {
    let c = program(
        r#"
fn ab() { return try({a: fail("A"), b: fail("B")}).error; }
fn ba() { return try({b: fail("B"), a: fail("A")}).error; }
"#,
    );
    assert_eq!(hash(&c, "p.rg", "ab"), hash(&c, "p.rg", "ba"));
    assert_eq!(run(&c, "p.rg", "ab", vec![]), run(&c, "p.rg", "ba", vec![]));
}

/// A field given twice is refused at parse time: with fields sorted for
/// hashing, "the last one wins" would depend on how the literal was written.
#[test]
fn duplicate_record_fields_are_refused() {
    let e = resolve::load_workspace_from_sources(&src(&[("p.rg", "fn f() { return {a: 1, b: 2, a: 3}; }")]))
        .err()
        .expect("should be refused");
    assert_eq!(e[0].code, "E0101");
    assert!(e[0].message.contains("field 'a' is given twice"), "{}", e[0].message);
}

/// A function value is its hash. Two definitions with one hash are one
/// function: equal, members of each other's dyn sets, shown the same way.
#[test]
fn function_identity_is_the_hash() {
    let c = program(
        r#"
fn f(x) { return x + 1; }
fn g(y) { return y + 1; }
fn twins() { return &f == &g; }
fn self_eq() { return &f == &f; }
fn through_dyn() { return dyn { f }(&g, 41); }
fn shown() { return [&f, &g]; }
"#,
    );
    assert_eq!(hash(&c, "p.rg", "twins"), hash(&c, "p.rg", "self_eq"));
    assert_eq!(run(&c, "p.rg", "twins", vec![]), "1");
    assert_eq!(run(&c, "p.rg", "self_eq", vec![]), "1");
    assert_eq!(run(&c, "p.rg", "through_dyn", vec![]), "42");
    let shown = run(&c, "p.rg", "shown", vec![]);
    assert!(!shown.contains('f') || !shown.contains("#f"), "{}", shown);
    let parts: Vec<&str> = shown.trim_matches(|c| c == '[' || c == ']').split(", ").collect();
    assert_eq!(parts[0], parts[1]);
}

/// Names are not part of identity, so they cannot be part of output: a
/// renamed definition prints, fails and dispatches exactly as before.
#[test]
fn renaming_changes_no_hash_and_no_output() {
    let before = r#"
fn double(x) { return x * 2; }
fn triple(x) { return x * 3; }
table ops { "d" -> double, "t" -> triple }
fn report(k) { return [&double, try(ops[k](1)), try(dyn { double }(&triple, 1))]; }
"#;
    let after = before
        .replace("double", "twice")
        .replace("triple", "thrice")
        .replace("ops", "routes")
        .replace("report", "summary")
        .replace("(x)", "(n)")
        .replace("x *", "n *");
    let a = program(before);
    let b = program(&after);
    assert_eq!(hash(&a, "p.rg", "report"), hash(&b, "p.rg", "summary"));
    for k in ["d", "t", "missing"] {
        let arg = || vec![eval::Value::Str(k.to_string())];
        assert_eq!(run(&a, "p.rg", "report", arg()), run(&b, "p.rg", "summary", arg()));
    }
}

/// `memo` keys and stores floats by their bits. -0.0 and 0.0 compare equal
/// but are different arguments (1.0 / x tells them apart); the cache once
/// merged them and answered +inf for -0.0, even on a later run.
#[test]
fn memo_keeps_the_sign_of_zero() {
    let dir = scratch("memo");
    let c = program(
        r#"
memo fn inv(x) { return 1.0 / x; }
memo fn negate(x) { return -x; }
fn both() { return [inv(0.0), inv(0.0 * (0.0 - 1.0))]; }
fn sign() { return 1.0 / negate(0.0); }
"#,
    );
    for _ in 0..2 {
        // a fresh engine each time: the second round reads the persisted cache
        let eng = eval::Engine::new(&c.ws).with_memo(&c.hashes, &dir);
        let both = eng.call(def(&c, "p.rg", "both"), vec![]).unwrap();
        assert_eq!(eval::show(&c.ws, &both), "[(1.0 / 0.0), (-1.0 / 0.0)]");
        let sign = eng.call(def(&c, "p.rg", "sign"), vec![]).unwrap();
        assert_eq!(eval::show(&c.ws, &sign), "(-1.0 / 0.0)");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Integers are 64-bit and checked. They used to wrap in the VM, grow
/// without bound in JavaScript, and panic the VM on MIN / -1.
#[test]
fn integer_arithmetic_is_checked() {
    let c = program(
        r#"
fn max() { return 9223372036854775807; }
fn min() { return 0 - max() - 1; }
fn add() { return max() + 1; }
fn sub() { return min() - 1; }
fn mul() { return max() * 2; }
fn div() { return min() / (0 - 1); }
fn rem() { return min() % (0 - 1); }
fn neg() { return -min(); }
fn caught() { return try(min() / (0 - 1)).ok; }
fn from_float() { return int(float(max()) * 2.0); }
fn from_str() { return try(int("9223372036854775808")).ok; }
"#,
    );
    let r = |n: &str| run(&c, "p.rg", n, vec![]);
    assert_eq!(r("add"), r#"error: "integer overflow: 9223372036854775807 + 1""#);
    assert_eq!(r("sub"), r#"error: "integer overflow: -9223372036854775808 - 1""#);
    assert_eq!(r("mul"), r#"error: "integer overflow: 9223372036854775807 * 2""#);
    assert_eq!(r("div"), r#"error: "integer overflow: -9223372036854775808 / -1""#);
    assert_eq!(r("rem"), "0");
    assert_eq!(r("neg"), r#"error: "integer overflow: -(-9223372036854775808)""#);
    assert_eq!(r("caught"), "0");
    assert!(r("from_float").contains("out of range"), "{}", r("from_float"));
    assert_eq!(r("from_str"), "0");
}

/// Comparing an Int with a Float is exact. Rounding the Int first made
/// 2^53 + 1 equal to 2^53.0, and equality stopped being transitive.
#[test]
fn mixed_comparison_is_exact() {
    let c = program(
        r#"
fn eq() { return 9007199254740993 == 9007199254740992.0; }
fn gt() { return 9007199254740993 > 9007199254740992.0; }
fn transitive() {
  let a = 9007199254740993;
  let b = 9007199254740992;
  let f = 9007199254740992.0;
  return [a == f, b == f, a == b];
}
fn nan() { let n = 0.0 / 0.0; return [1 == n, 1 != n, 1 < n, 1 >= n]; }
fn huge() { return [9223372036854775807 < 9223372036854775808.0, 1 == 1.0, 2 > 1.5]; }
"#,
    );
    let r = |n: &str| run(&c, "p.rg", n, vec![]);
    assert_eq!(r("eq"), "0");
    assert_eq!(r("gt"), "1");
    assert_eq!(r("transitive"), "[0, 1, 0]");
    assert_eq!(r("nan"), "[0, 1, 0, 0]");
    assert_eq!(r("huge"), "[1, 1, 1]");
}

/// A recursion too deep for any stack is an ordinary, catchable error at a
/// fixed depth — not an abort of the whole process.
#[test]
fn deep_recursion_is_a_catchable_error() {
    let dir = scratch("depth");
    std::fs::write(
        dir.join("r.rg"),
        "fn down(n) { if (n == 0) { return 0; } return 1 + down(n - 1); }\n\
         fn safe(n) { return try(down(n)); }\n",
    )
    .unwrap();
    let out = Command::new(rigid_bin())
        .args(["run", "r.rg#safe", "1000000", "--root"])
        .arg(&dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{:?} {}", out.status, String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("call depth limit exceeded"), "{}", stdout);
    let out = Command::new(rigid_bin())
        .args(["run", "r.rg#safe", "5000", "--root"])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("value: 5000"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The parser bounds how deep a tree can be, so no later pass can be
/// driven off the end of the stack. Every source either checks or is E0101.
#[test]
fn nesting_is_bounded() {
    on_big_stack(|| {
        let refuse = |s: String| {
            let e = resolve::load_workspace_from_sources(&[("n.rg".to_string(), s)]).err();
            let e = e.expect("should be refused");
            assert!(e[0].message.contains("nested too deeply"), "{}", e[0].message);
        };
        refuse(format!("fn f() {{ return {}1{}; }}", "(".repeat(20_000), ")".repeat(20_000)));
        refuse(format!("fn f() {{ return 1{}; }}", " + 1".repeat(200_000)));
        refuse(format!("fn f() {{ return {}1; }}", "-".repeat(50_000)));
        refuse(format!(
            "fn f(x) {{ if (x == 0) {{ return 0; }}{} return 2; }}",
            " else if (x == 1) { return 1; }".repeat(50_000)
        ));
        // just under the limit: every pass, and the VM, handle it
        let c = load(&src(&[(
            "n.rg",
            &format!(
                "fn f() {{ return {}1{}; }}\nfn g(x) {{ return x{}; }}",
                "(".repeat(990),
                ")".repeat(990),
                " + x".repeat(990)
            ),
        )]));
        assert!(errors(&c).is_empty());
        assert_eq!(run(&c, "n.rg", "f", vec![]), "1");
        assert_eq!(run(&c, "n.rg", "g", vec![eval::Value::Int(1)]), "991");
    });
}

/// Loop analysis once re-walked each loop body twice per enclosing loop,
/// so 30 nested loops took longer than anyone would wait.
#[test]
fn nested_loops_analyze_in_linear_time() {
    on_big_stack(|| {
        let n = 400;
        let s = format!(
            "fn w() {{ let i = 0; {}i = i + 1;{} return i; }}",
            "while (i < 1) { ".repeat(n),
            " }".repeat(n)
        );
        let t = std::time::Instant::now();
        let c = load(&src(&[("w.rg", &s)]));
        assert!(errors(&c).is_empty());
        assert_eq!(run(&c, "w.rg", "w", vec![]), "1");
        assert!(t.elapsed().as_secs() < 20, "took {:?}", t.elapsed());
    });
}

/// Two modules whose names differ only in punctuation would mangle to one
/// JavaScript name, and one definition would silently replace the other.
#[test]
fn js_names_cannot_collide() {
    let c = load(&src(&[("a-b.rg", "fn f() { return 1; }"), ("a_b.rg", "fn f() { return 2; }")]));
    assert!(errors(&c).is_empty());
    let d = backend_js::check(&c.ws);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].code, "E0213");
}

/// The compiler's output for one input is one output. A capability rule
/// once named whichever definition a hash set yielded first.
#[test]
fn diagnostics_are_deterministic() {
    let dir = scratch("rules");
    std::fs::create_dir_all(dir.join("domain")).unwrap();
    std::fs::create_dir_all(dir.join("http")).unwrap();
    std::fs::write(
        dir.join("http/send.rg"),
        "extern fn post(url, body) !net = \"globalThis.fetch\";\nfn ship(u, b) !net { return post(u, b); }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("domain/order.rg"),
        "import { ship } from \"../http/send.rg\";\nfn total(items) !net { return ship(\"u\", str(len(items))); }\n",
    )
    .unwrap();
    std::fs::write(dir.join("rigid.rules"), "deny domain/** -> http/**\ndeny domain/** -> !net\n").unwrap();
    // the first check pins the import and says so; every later one must
    // print the same thing as every other
    Command::new(rigid_bin()).args(["check", "--root"]).arg(&dir).output().unwrap();
    let mut seen = Vec::new();
    for _ in 0..8 {
        let out = Command::new(rigid_bin()).args(["check", "--root"]).arg(&dir).output().unwrap();
        seen.push(String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout));
    }
    assert!(seen.iter().all(|s| s == &seen[0]), "{:#?}", seen);
    assert!(seen[0].contains("via http/send.rg#post"), "{}", seen[0]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The standard library's tests and properties, run as part of this suite.
/// Checked arithmetic found that isqrt(max) returned a negative number.
#[test]
fn standard_library_holds() {
    let std_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("std");
    let out = Command::new(rigid_bin())
        .args(["test", "--no-cache", "--root"])
        .arg(&std_dir)
        .env("RIGID_CACHE_DIR", scratch("stdcache"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(" 0 failed"), "{}", stdout);
}

// ------------------------------------------------------ program generator

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())]
    }
    fn shuffle<T>(&mut self, xs: &mut [T]) {
        for i in (1..xs.len()).rev() {
            xs.swap(i, self.below(i + 1));
        }
    }
}

const INTS: &[&str] = &[
    "0", "1", "2", "3", "7", "-1", "-5", "100", "9223372036854775807",
    "-9223372036854775807", "9007199254740993", "4611686018427387904",
];
const FLOATS: &[&str] = &[
    "0.0", "0.5", "1.0", "2.5", "0.1", "3.0", "1000000000000000000000.0", "0.0000001",
];
const STRS: &[&str] = &[
    "\"\"", "\"a\"", "\"ab\"", "\"é\"", "\"😀\"", "\"～\"", "\"constructor\"",
    "\"__proto__\"", "\"k0\"", "\"k1\"", "\" 12\"", "\"1.5\"", "\"inf\"", "\"x\\ny\"", "\"-0\"",
];
const FIELDS: &[&str] = &["a", "b", "c", "ok", "value", "constructor", "__proto__", "toString"];
const OPS: &[&str] = &["+", "-", "*", "/", "%", "==", "!=", "<", ">", "<=", ">="];
const KEYS: &[&str] = &["k0", "k1", "k2"];

#[derive(Clone)]
enum E {
    Lit(&'static str),
    Local(usize),
    Bin(&'static str, Box<E>, Box<E>),
    Neg(Box<E>),
    If(Box<E>, Box<E>, Box<E>),
    List(Vec<E>),
    Rec(Vec<(&'static str, E)>),
    Get(Box<E>, &'static str),
    Index(Box<E>, Box<E>),
    Un(&'static str, Box<E>),
    Bi(&'static str, Box<E>, Box<E>),
    Substr(Box<E>, Box<E>, Box<E>),
    Call(usize, Vec<E>),
    Ref(usize),
    Dyn(Vec<usize>, Box<E>, Vec<E>),
    Table(usize, &'static str, Vec<E>),
    /// call the closure held in a local; .0 is the lambda's ordinal
    DynLam(usize, usize, Vec<E>),
}

#[derive(Clone)]
enum S {
    Let(usize, E),
    Assign(usize, E),
    If(E, Vec<S>, Vec<S>),
    While(usize, i64, Vec<S>),
    For(usize, E, Vec<S>),
    Ret(E),
    Ex(E),
    /// let local = fn(params) { return body; };
    Lambda(usize, Vec<usize>, E),
}

struct Fun {
    params: usize,
    memo: bool,
    body: Vec<S>,
}

struct Table {
    arity: usize,
    entries: Vec<(&'static str, usize)>,
}

struct Prog {
    helpers: Vec<Fun>,
    tables: Vec<Table>,
    main: Fun,
}

struct FnGen<'a> {
    rng: &'a mut Rng,
    /// helper arities; this function may call/reference helpers below `limit`
    arities: &'a [usize],
    limit: usize,
    tables: &'a [Table],
    scope: Vec<usize>,
    assignable: Vec<usize>,
    next_local: usize,
    /// (ordinal, arity, holding local) of lambdas made so far
    lambdas: Vec<(usize, usize, usize)>,
    /// local -> the domain the checker will know it by
    doms: HashMap<usize, D>,
}

/// Value domains, as the checker's E0212 inference sees them.
#[derive(Clone, Copy, PartialEq, Debug)]
enum D {
    Any,
    Num,
    Str,
    List,
    Rec,
}

impl<'a> FnGen<'a> {
    fn fresh(&mut self) -> usize {
        self.next_local += 1;
        self.next_local - 1
    }

    /// A local whose known domain fits `want`, if there is one.
    fn local_of(&mut self, want: D) -> Option<E> {
        let fits: Vec<usize> = self
            .scope
            .iter()
            .copied()
            .filter(|v| want == D::Any || matches!(self.doms.get(v), None | Some(D::Any)) || self.doms.get(v) == Some(&want))
            .collect();
        if fits.is_empty() {
            None
        } else {
            Some(E::Local(self.rng.pick(&fits)))
        }
    }

    fn leaf(&mut self, want: D) -> E {
        if self.rng.chance(45) {
            if let Some(l) = self.local_of(want) {
                return l;
            }
        }
        match want {
            D::Num => E::Lit(if self.rng.chance(25) { self.rng.pick(FLOATS) } else { self.rng.pick(INTS) }),
            D::Str => E::Lit(self.rng.pick(STRS)),
            D::List => E::List(vec![]),
            D::Rec => E::Rec(vec![]),
            D::Any => match self.rng.below(10) {
                0 if self.limit > 0 => E::Ref(self.rng.below(self.limit)),
                1 => E::Lit(self.rng.pick(FLOATS)),
                2 | 3 => E::Lit(self.rng.pick(STRS)),
                _ => E::Lit(self.rng.pick(INTS)),
            },
        }
    }

    fn args(&mut self, n: usize, depth: u32) -> Vec<E> {
        // arguments lean numeric: parameters used in arithmetic are inferred
        // integer, and a literal of another domain would be E0212
        (0..n)
            .map(|_| {
                let w = if self.rng.chance(70) { D::Num } else { D::Any };
                self.expr(depth, w)
            })
            .collect()
    }

    /// An expression whose domain, as far as the checker can see, is `want`
    /// (or unknown). Unknowable values — calls, dyn, tables, fields — still
    /// fail at runtime sometimes, which is what the engines are compared on.
    fn expr(&mut self, depth: u32, want: D) -> E {
        if depth == 0 || self.rng.chance(25) {
            return self.leaf(want);
        }
        let d = depth - 1;
        let b = |e: E| Box::new(e);
        let pick = self.rng.below(24);
        // forms whose result domain is known, by the domain they produce
        let produces = |k: usize| -> D {
            match k {
                0..=3 | 4 | 10 => D::Num, // arithmetic, negation, len/int/float
                6 | 14 => D::List,        // list literal, push
                7 | 11 | 12 => D::Rec,    // record literal, try
                15 | 21 => D::Str,        // substr, str/join
                _ => D::Any,
            }
        };
        let p = produces(pick);
        if want != D::Any && p != D::Any && p != want {
            return self.leaf(want);
        }
        match pick {
            0..=3 => {
                let op = self.rng.pick(OPS);
                if op == "==" || op == "!=" {
                    E::Bin(op, b(self.expr(d, D::Any)), b(self.expr(d, D::Any)))
                } else if op == "+" && want == D::Any && self.rng.chance(30) {
                    let w = if self.rng.chance(50) { D::Str } else { D::List };
                    E::Bin(op, b(self.expr(d, w)), b(self.expr(d, w)))
                } else {
                    E::Bin(op, b(self.expr(d, D::Num)), b(self.expr(d, D::Num)))
                }
            }
            4 => E::Neg(b(self.expr(d, D::Num))),
            5 => E::If(b(self.expr(d, D::Num)), b(self.expr(d, want)), b(self.expr(d, want))),
            6 => {
                let n = self.rng.below(4);
                E::List((0..n).map(|_| self.expr(d, D::Any)).collect())
            }
            7 => {
                // distinct names: a field given twice is a parse error
                let mut names: Vec<&'static str> = FIELDS.to_vec();
                self.rng.shuffle(&mut names);
                names.truncate(self.rng.below(4));
                let fs = names.into_iter().map(|f| (f, self.expr(d, D::Any))).collect();
                E::Rec(fs)
            }
            8 => E::Get(b(self.expr(d, D::Rec)), self.rng.pick(FIELDS)),
            9 => {
                let w = if self.rng.chance(70) { D::List } else { D::Str };
                E::Index(b(self.expr(d, w)), b(self.expr(d, D::Num)))
            }
            10 => match self.rng.below(3) {
                0 => {
                    let w = if self.rng.chance(50) { D::List } else { D::Str };
                    E::Un("len", b(self.expr(d, w)))
                }
                1 => E::Un("int", b(self.expr(d, D::Any))),
                _ => E::Un("float", b(self.expr(d, D::Any))),
            },
            11 | 12 => E::Un("try", b(self.expr(d, D::Any))),
            13 if self.rng.chance(40) => E::Un("fail", b(self.expr(d, D::Any))),
            14 => E::Bi("push", b(self.expr(d, D::List)), b(self.expr(d, D::Any))),
            15 => E::Substr(b(self.expr(d, D::Str)), b(self.expr(d, D::Num)), b(self.expr(d, D::Num))),
            16 | 17 if self.limit > 0 => {
                let h = self.rng.below(self.limit);
                let n = self.arities[h];
                E::Call(h, self.args(n, d))
            }
            18 if self.limit > 0 => {
                // a dyn site over helpers of one arity; the callee is usually
                // a member, sometimes not (a quarantine violation)
                let h = self.rng.below(self.limit);
                let n = self.arities[h];
                let mut set: Vec<usize> = (0..self.limit).filter(|&x| self.arities[x] == n).collect();
                self.rng.shuffle(&mut set);
                set.truncate(1 + self.rng.below(set.len()));
                let callee = if self.rng.chance(80) {
                    E::Ref(self.rng.pick(&set))
                } else {
                    self.leaf(D::Any)
                };
                E::Dyn(set, b(callee), self.args(n, d))
            }
            19 => {
                let usable: Vec<usize> = (0..self.tables.len())
                    .filter(|&t| self.tables[t].entries.iter().all(|&(_, h)| h < self.limit))
                    .collect();
                if usable.is_empty() {
                    return self.leaf(want);
                }
                let t = self.rng.pick(&usable);
                let key = if self.rng.chance(80) { self.rng.pick(KEYS) } else { "zz" };
                let n = self.tables[t].arity;
                E::Table(t, key, self.args(n, d))
            }
            20 if !self.lambdas.is_empty() => {
                let (ord, arity, local) = self.rng.pick(&self.lambdas);
                E::DynLam(ord, local, self.args(arity, d))
            }
            21 => {
                if self.rng.chance(50) {
                    E::Un("str", b(self.expr(d, D::Num)))
                } else {
                    E::Bi("join", b(self.expr(d, D::List)), b(self.expr(d, D::Str)))
                }
            }
            22 => {
                let w = if self.rng.chance(50) { D::Str } else { D::Any };
                E::Un("chars", b(self.expr(d, w)))
            }
            _ => self.leaf(want),
        }
    }

    /// The domain the checker will infer for a local initialized by `e`.
    fn domain_of(&self, e: &E) -> D {
        match e {
            E::Lit(s) if s.starts_with('"') => D::Str,
            E::Lit(_) => D::Num,
            E::List(_) | E::Bi("push", ..) | E::Un("chars", _) => D::List,
            E::Rec(_) | E::Un("try", _) => D::Rec,
            E::Substr(..) | E::Un("str", _) | E::Bi("join", ..) => D::Str,
            E::Bin(op, ..) if !matches!(*op, "==" | "!=" | "+") => D::Num,
            E::Neg(_) | E::Un("len", _) | E::Un("int", _) | E::Un("float", _) => D::Num,
            E::Local(v) => self.doms.get(v).copied().unwrap_or(D::Any),
            _ => D::Any,
        }
    }

    fn block(&mut self, depth: u32, n: usize, top: bool) -> Vec<S> {
        (0..n).map(|_| self.stmt(depth, top)).collect()
    }

    fn stmt(&mut self, depth: u32, top: bool) -> S {
        match self.rng.below(16) {
            0..=4 => {
                let e = self.expr(3, D::Any);
                // caught often, so a run gets past its first failure
                let e = if self.rng.chance(30) { E::Un("try", Box::new(e)) } else { e };
                let v = self.fresh();
                let dm = self.domain_of(&e);
                self.doms.insert(v, dm);
                self.scope.push(v);
                self.assignable.push(v);
                S::Let(v, e)
            }
            5 | 6 if !self.assignable.is_empty() => {
                // keep the local's domain, mostly; a conflicting assignment
                // silences the checker for that slot, which is also fine
                let v = self.rng.pick(&self.assignable);
                let want = self.doms.get(&v).copied().unwrap_or(D::Any);
                let want = if self.rng.chance(85) { want } else { D::Any };
                let e = self.expr(3, want);
                if self.domain_of(&e) != want {
                    self.doms.insert(v, D::Any);
                }
                S::Assign(v, e)
            }
            7 if depth > 0 => {
                let c = self.expr(2, D::Num);
                let n = 1 + self.rng.below(3);
                let t = self.block(depth - 1, n, false);
                let e = if self.rng.chance(50) {
                    let n = self.rng.below(3);
                    self.block(depth - 1, n, false)
                } else {
                    vec![]
                };
                S::If(c, t, e)
            }
            8 if depth > 0 => {
                // the counter is readable but never assigned by the body, so
                // the loop ends
                let c = self.fresh();
                self.scope.push(c);
                let bound = 1 + self.rng.below(4) as i64;
                let n = 1 + self.rng.below(3);
                let body = self.block(depth - 1, n, false);
                S::While(c, bound, body)
            }
            9 if depth > 0 => {
                let it = if self.rng.chance(70) {
                    let n = self.rng.below(4);
                    E::List(self.args(n, 1))
                } else {
                    self.expr(2, D::Any)
                };
                let v = self.fresh();
                self.scope.push(v);
                let n = 1 + self.rng.below(3);
                let body = self.block(depth - 1, n, false);
                S::For(v, it, body)
            }
            10 if top && !self.scope.is_empty() => {
                // a closure over locals already in scope
                let n = self.rng.below(3);
                let params: Vec<usize> = (0..n).map(|_| self.fresh()).collect();
                let outer = self.scope.clone();
                self.scope.extend(params.iter().copied());
                let body = self.expr(3, D::Any);
                self.scope = outer;
                let v = self.fresh();
                let ord = self.lambdas.len();
                self.lambdas.push((ord, n, v));
                self.scope.push(v);
                S::Lambda(v, params, body)
            }
            11 if self.rng.chance(30) => S::Ret(self.expr(3, D::Any)),
            _ => {
                let e = self.expr(3, D::Any);
                S::Ex(if self.rng.chance(75) { E::Un("try", Box::new(e)) } else { e })
            }
        }
    }
}

fn gen_fun(rng: &mut Rng, arities: &[usize], limit: usize, tables: &[Table], params: usize) -> Fun {
    let mut g = FnGen {
        rng,
        arities,
        limit,
        tables,
        scope: (0..params).collect(),
        assignable: (0..params).collect(),
        next_local: params,
        lambdas: vec![],
        doms: HashMap::new(),
    };
    let n = 1 + g.rng.below(5);
    let mut body = g.block(2, n, true);
    body.push(S::Ret(g.expr(3, D::Any)));
    let memo = g.rng.chance(10);
    Fun { params, memo, body }
}

fn gen_prog(rng: &mut Rng) -> Prog {
    let nh = rng.below(5);
    let arities: Vec<usize> = (0..nh).map(|_| rng.below(3)).collect();
    // tables over existing helpers, all entries of one arity
    let mut tables = Vec::new();
    for a in 0..3 {
        let members: Vec<usize> = (0..nh).filter(|&h| arities[h] == a).collect();
        if !members.is_empty() && rng.chance(50) {
            let entries = KEYS.iter().take(1 + rng.below(KEYS.len())).map(|&k| (k, rng.pick(&members))).collect();
            tables.push(Table { arity: a, entries });
        }
    }
    let helpers = (0..nh).map(|h| gen_fun(rng, &arities, h, &tables, arities[h])).collect();
    let main = gen_fun(rng, &arities, nh, &tables, 0);
    Prog { helpers, tables, main }
}

// ------------------------------------------------------------- rendering

/// One way of writing a program down. Two styles of one program must mean
/// the same thing: every difference between them is one the canonical form
/// erases.
struct Style {
    rng: Rng,
    /// local id -> the number in its name
    perm: Vec<usize>,
    renamed: bool,
}

impl Style {
    fn local(&self, v: usize) -> String {
        if self.renamed {
            format!("w_{}", self.perm[v])
        } else {
            format!("v{}", v)
        }
    }
    fn helper(&self, h: usize) -> String {
        if self.renamed { format!("g_{}", 9 - h) } else { format!("h{}", h) }
    }
    fn table(&self, t: usize) -> String {
        if self.renamed { format!("tab_{}", t) } else { format!("t{}", t) }
    }
    fn main(&self) -> &'static str {
        if self.renamed { "entry" } else { "main" }
    }
    fn gap(&mut self) -> &'static str {
        if self.renamed && self.rng.chance(30) { "\n      " } else { " " }
    }

    fn expr(&mut self, e: &E) -> String {
        match e {
            E::Lit(s) => s.to_string(),
            E::Local(v) => self.local(*v),
            E::Bin(op, l, r) => {
                let (l, g, r) = (self.expr(l), self.gap(), self.expr(r));
                format!("({} {}{}{})", l, op, g, r)
            }
            E::Neg(x) => format!("-({})", self.expr(x)),
            E::If(c, t, x) => format!("(if ({}) {} else {})", self.expr(c), self.expr(t), self.expr(x)),
            E::List(xs) => {
                let xs: Vec<String> = xs.iter().map(|x| self.expr(x)).collect();
                format!("[{}]", xs.join(", "))
            }
            E::Rec(fs) => {
                let mut parts: Vec<String> = fs.iter().map(|(n, v)| format!("{}: {}", n, self.expr(v))).collect();
                if self.renamed {
                    self.rng.shuffle(&mut parts);
                }
                format!("{{{}}}", parts.join(", "))
            }
            E::Get(x, f) => format!("({}).{}", self.expr(x), f),
            E::Index(x, k) => format!("({})[{}]", self.expr(x), self.expr(k)),
            E::Un(f, x) => format!("{}({})", f, self.expr(x)),
            E::Bi(f, a, b) => format!("{}({}, {})", f, self.expr(a), self.expr(b)),
            E::Substr(a, b, c) => format!("substr({}, {}, {})", self.expr(a), self.expr(b), self.expr(c)),
            E::Call(h, args) => {
                let a: Vec<String> = args.iter().map(|x| self.expr(x)).collect();
                format!("{}({})", self.helper(*h), a.join(", "))
            }
            E::Ref(h) => format!("&{}", self.helper(*h)),
            E::Dyn(set, callee, args) => {
                let mut names: Vec<String> = set.iter().map(|&h| self.helper(h)).collect();
                if self.renamed {
                    self.rng.shuffle(&mut names);
                }
                let mut a = vec![self.expr(callee)];
                a.extend(args.iter().map(|x| self.expr(x)));
                format!("dyn {{ {} }}({})", names.join(", "), a.join(", "))
            }
            E::Table(t, k, args) => {
                let a: Vec<String> = args.iter().map(|x| self.expr(x)).collect();
                format!("{}[\"{}\"]({})", self.table(*t), k, a.join(", "))
            }
            E::DynLam(..) => unreachable!("rendered by fun(), which knows the owner"),
        }
    }

    fn expr_in(&mut self, owner: &str, e: &E) -> String {
        // DynLam needs the owner's name; everything else renders the same.
        match e {
            E::DynLam(ord, local, args) => {
                let mut a = vec![self.local(*local)];
                a.extend(args.iter().map(|x| self.expr_in(owner, x)));
                format!("dyn {{ {}${} }}({})", owner, ord, a.join(", "))
            }
            E::Bin(op, l, r) => format!("({} {} {})", self.expr_in(owner, l), op, self.expr_in(owner, r)),
            E::Neg(x) => format!("-({})", self.expr_in(owner, x)),
            E::If(c, t, x) => format!(
                "(if ({}) {} else {})",
                self.expr_in(owner, c),
                self.expr_in(owner, t),
                self.expr_in(owner, x)
            ),
            E::List(xs) => {
                let xs: Vec<String> = xs.iter().map(|x| self.expr_in(owner, x)).collect();
                format!("[{}]", xs.join(", "))
            }
            E::Rec(fs) => {
                let mut parts: Vec<String> =
                    fs.iter().map(|(n, v)| format!("{}: {}", n, self.expr_in(owner, v))).collect();
                if self.renamed {
                    self.rng.shuffle(&mut parts);
                }
                format!("{{{}}}", parts.join(", "))
            }
            E::Get(x, f) => format!("({}).{}", self.expr_in(owner, x), f),
            E::Index(x, k) => format!("({})[{}]", self.expr_in(owner, x), self.expr_in(owner, k)),
            E::Un(f, x) => format!("{}({})", f, self.expr_in(owner, x)),
            E::Bi(f, a, b) => format!("{}({}, {})", f, self.expr_in(owner, a), self.expr_in(owner, b)),
            E::Substr(a, b, c) => format!(
                "substr({}, {}, {})",
                self.expr_in(owner, a),
                self.expr_in(owner, b),
                self.expr_in(owner, c)
            ),
            E::Call(h, args) => {
                let a: Vec<String> = args.iter().map(|x| self.expr_in(owner, x)).collect();
                format!("{}({})", self.helper(*h), a.join(", "))
            }
            E::Dyn(set, callee, args) => {
                let mut names: Vec<String> = set.iter().map(|&h| self.helper(h)).collect();
                if self.renamed {
                    self.rng.shuffle(&mut names);
                }
                let mut a = vec![self.expr_in(owner, callee)];
                a.extend(args.iter().map(|x| self.expr_in(owner, x)));
                format!("dyn {{ {} }}({})", names.join(", "), a.join(", "))
            }
            E::Table(t, k, args) => {
                let a: Vec<String> = args.iter().map(|x| self.expr_in(owner, x)).collect();
                format!("{}[\"{}\"]({})", self.table(*t), k, a.join(", "))
            }
            other => self.expr(other),
        }
    }

    fn stmts(&mut self, owner: &str, ss: &[S], ind: usize, out: &mut String) {
        let pad = "  ".repeat(ind);
        for s in ss {
            let note = if self.renamed && self.rng.chance(20) { "  // note" } else { "" };
            match s {
                S::Let(v, e) => {
                    let e = self.expr_in(owner, e);
                    out.push_str(&format!("{}let {} = {};{}\n", pad, self.local(*v), e, note));
                }
                S::Assign(v, e) => {
                    let e = self.expr_in(owner, e);
                    out.push_str(&format!("{}{} = {};{}\n", pad, self.local(*v), e, note));
                }
                S::Ret(e) => {
                    let e = self.expr_in(owner, e);
                    out.push_str(&format!("{}return {};{}\n", pad, e, note));
                }
                S::Ex(e) => {
                    let e = self.expr_in(owner, e);
                    out.push_str(&format!("{}{};{}\n", pad, e, note));
                }
                S::If(c, t, e) => {
                    let c = self.expr_in(owner, c);
                    out.push_str(&format!("{}if ({}) {{\n", pad, c));
                    self.stmts(owner, t, ind + 1, out);
                    if e.is_empty() {
                        out.push_str(&format!("{}}}\n", pad));
                    } else {
                        out.push_str(&format!("{}}} else {{\n", pad));
                        self.stmts(owner, e, ind + 1, out);
                        out.push_str(&format!("{}}}\n", pad));
                    }
                }
                S::While(c, bound, body) => {
                    let c = self.local(*c);
                    out.push_str(&format!("{}let {} = 0;\n{}while ({} < {}) {{\n", pad, c, pad, c, bound));
                    self.stmts(owner, body, ind + 1, out);
                    out.push_str(&format!("{}  {} = {} + 1;\n{}}}\n", pad, c, c, pad));
                }
                S::For(v, it, body) => {
                    let it = self.expr_in(owner, it);
                    out.push_str(&format!("{}for ({} in {}) {{\n", pad, self.local(*v), it));
                    self.stmts(owner, body, ind + 1, out);
                    out.push_str(&format!("{}}}\n", pad));
                }
                S::Lambda(v, params, body) => {
                    let ps: Vec<String> = params.iter().map(|&p| self.local(p)).collect();
                    let body = self.expr_in(owner, body);
                    out.push_str(&format!(
                        "{}let {} = fn({}) {{ return {}; }};{}\n",
                        pad,
                        self.local(*v),
                        ps.join(", "),
                        body,
                        note
                    ));
                }
            }
        }
    }

    fn fun(&mut self, name: &str, f: &Fun) -> String {
        let ps: Vec<String> = (0..f.params).map(|p| self.local(p)).collect();
        let mut out = format!("{}fn {}({}) {{\n", if f.memo { "memo " } else { "" }, name, ps.join(", "));
        self.stmts(name, &f.body, 1, &mut out);
        out.push_str("}\n\n");
        out
    }

    fn prog(&mut self, p: &Prog) -> String {
        let mut out = String::new();
        if self.renamed {
            out.push_str("// the same program, written differently\n\n");
        }
        // definition order is not identity either
        let mut order: Vec<usize> = (0..p.helpers.len()).collect();
        if self.renamed {
            order.reverse();
        }
        for h in order {
            let name = self.helper(h);
            out.push_str(&self.fun(&name, &p.helpers[h]));
        }
        for (t, tab) in p.tables.iter().enumerate() {
            let mut es: Vec<String> =
                tab.entries.iter().map(|(k, h)| format!("\"{}\" -> {}", k, self.helper(*h))).collect();
            if self.renamed {
                self.rng.shuffle(&mut es);
            }
            out.push_str(&format!("table {} {{ {} }}\n\n", self.table(t), es.join(", ")));
        }
        let main = self.main();
        out.push_str(&self.fun(main, &p.main));
        out
    }
}

fn count_locals(f: &Fun) -> usize {
    fn s(x: &S, m: &mut usize) {
        match x {
            S::Let(v, _) | S::Assign(v, _) => *m = (*m).max(v + 1),
            S::While(v, _, b) | S::For(v, _, b) => {
                *m = (*m).max(v + 1);
                b.iter().for_each(|y| s(y, m));
            }
            S::If(_, t, e) => t.iter().chain(e.iter()).for_each(|y| s(y, m)),
            S::Lambda(v, ps, _) => {
                *m = (*m).max(v + 1);
                ps.iter().for_each(|p| *m = (*m).max(p + 1));
            }
            S::Ret(_) | S::Ex(_) => {}
        }
    }
    let mut m = f.params;
    f.body.iter().for_each(|x| s(x, &mut m));
    m
}

// ----------------------------------------------------------------- the test

struct Case {
    a_path: String,
    b_path: String,
    /// (name in A, name in B) for every definition, lifted lambdas included
    roles: Vec<(String, String)>,
}

fn env_num(k: &str, default: u64) -> u64 {
    std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[test]
fn generated_programs_keep_every_guarantee() {
    on_big_stack(generated_programs);
}

fn generated_programs() {
    let cases = env_num("RIGID_FUZZ_CASES", 300) as usize;
    let seed = env_num("RIGID_FUZZ_SEED", 0x5eed_0f_71_91d);
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);

    let mut a_src = Vec::new();
    let mut b_src = Vec::new();
    let mut meta = Vec::new();
    for i in 0..cases {
        let p = gen_prog(&mut rng);
        let locals = p.helpers.iter().chain(std::iter::once(&p.main)).map(count_locals).max().unwrap_or(0);
        let mut perm: Vec<usize> = (0..locals).collect();
        rng.shuffle(&mut perm);
        let mut a = Style { rng: Rng(seed ^ (i as u64 + 1)), perm: perm.clone(), renamed: false };
        let mut b = Style { rng: Rng(seed ^ (i as u64 + 7919)), perm, renamed: true };
        let a_path = format!("a/p{}.rg", i);
        let b_path = format!("b/q{}.rg", i);
        a_src.push((a_path.clone(), a.prog(&p)));
        b_src.push((b_path.clone(), b.prog(&p)));
        let mut roles: Vec<(String, String)> = Vec::new();
        let lams = |f: &Fun| f.body.iter().filter(|s| matches!(s, S::Lambda(..))).count();
        for (h, f) in p.helpers.iter().enumerate() {
            roles.push((a.helper(h), b.helper(h)));
            for k in 0..lams(f) {
                roles.push((format!("{}${}", a.helper(h), k), format!("{}${}", b.helper(h), k)));
            }
        }
        for t in 0..p.tables.len() {
            roles.push((a.table(t), b.table(t)));
        }
        roles.push(("main".into(), "entry".into()));
        for k in 0..lams(&p.main) {
            roles.push((format!("main${}", k), format!("entry${}", k)));
        }
        meta.push(Case { a_path, b_path, roles });
    }

    let a = load(&a_src);
    let b = load(&b_src);

    // The checker itself must not care how a program is written.
    let codes = |c: &Checked, path: &str| -> Vec<&'static str> {
        let mut v: Vec<&'static str> = c
            .diags
            .iter()
            .filter(|d| d.severity == Severity::Error && d.file == path)
            .map(|d| d.code)
            .collect();
        v.sort();
        v
    };

    let mut good: Vec<usize> = Vec::new();
    let mut values = 0;
    let mut failures: Vec<String> = Vec::new();
    for (i, m) in meta.iter().enumerate() {
        let (ea, eb) = (codes(&a, &m.a_path), codes(&b, &m.b_path));
        if ea != eb {
            failures.push(format!(
                "program {}: diagnostics differ between spellings: {:?} vs {:?}\n--- A ---\n{}\n--- B ---\n{}",
                i, ea, eb, a_src[i].1, b_src[i].1
            ));
            continue;
        }
        if !ea.is_empty() {
            continue; // rejected identically by both: nothing to run
        }
        // 1. every definition keeps its hash
        for (na, nb) in &m.roles {
            let (ha, hb) = (hash(&a, &m.a_path, na), hash(&b, &m.b_path, nb));
            if ha != hb {
                failures.push(format!(
                    "program {}: {} and {} hash differently\n--- A ---\n{}\n--- B ---\n{}",
                    i, na, nb, a_src[i].1, b_src[i].1
                ));
            }
        }
        // ... and its behaviour
        let (ra, rb) = (run(&a, &m.a_path, "main", vec![]), run(&b, &m.b_path, "entry", vec![]));
        if !ra.starts_with("error: ") {
            values += 1;
        }
        if ra != rb {
            failures.push(format!(
                "program {}: same hash, different results:\n  A: {}\n  B: {}\n--- A ---\n{}\n--- B ---\n{}",
                i, ra, rb, a_src[i].1, b_src[i].1
            ));
        }
        good.push(i);
    }
    if std::env::var("RIGID_FUZZ_DEBUG").is_ok() {
        let mut tally: HashMap<String, usize> = HashMap::new();
        for d in a.diags.iter().filter(|d| d.severity == Severity::Error) {
            *tally.entry(format!("{} {}", d.code, d.message.split(':').next().unwrap_or(""))).or_default() += 1;
        }
        let mut t: Vec<_> = tally.into_iter().collect();
        t.sort_by(|x, y| y.1.cmp(&x.1));
        for (k, n) in t.iter().take(15) {
            eprintln!("{:5}  {}", n, k);
        }
    }
    assert!(
        good.len() * 2 >= cases,
        "the generator is producing mostly rejected programs ({} of {} usable)",
        good.len(),
        cases
    );

    // 2. the JavaScript backend agrees with the VM
    if node_available() {
        let only: Vec<(String, String)> = good.iter().map(|&i| a_src[i].clone()).collect();
        let c = load(&only);
        assert!(errors(&c).is_empty(), "{:?}", errors(&c));
        assert!(backend_js::check(&c.ws).is_empty());
        let dir = scratch("js");
        std::fs::write(dir.join("out.js"), backend_js::Js::emit(&c.ws)).unwrap();
        std::fs::write(
            dir.join("driver.js"),
            r#"const m = require("./out.js");
const rt = m.$rigid;
const refs = JSON.parse(require("fs").readFileSync(process.argv[2], "utf8"));
const out = [];
for (const r of refs) {
  let line;
  try { line = rt.disp(rt.byref[r]()); } catch (e) { line = "error: " + rt.disp(rt.caught(e)); }
  out.push(line);
}
process.stdout.write(JSON.stringify(out));
"#,
        )
        .unwrap();
        let refs: Vec<String> = good.iter().map(|&i| format!("{}#main", meta[i].a_path)).collect();
        std::fs::write(dir.join("refs.json"), serde_json::to_string(&refs).unwrap()).unwrap();
        let out = Command::new("node")
            .arg(dir.join("driver.js"))
            .arg(dir.join("refs.json"))
            .output()
            .unwrap();
        assert!(out.status.success(), "node failed: {}", String::from_utf8_lossy(&out.stderr));
        let js: Vec<String> = serde_json::from_slice(&out.stdout).unwrap();
        for (k, &i) in good.iter().enumerate() {
            let vm = run(&c, &meta[i].a_path, "main", vec![]);
            if vm != js[k] {
                failures.push(format!(
                    "program {}: the VM and JavaScript disagree:\n  VM: {}\n  JS: {}\n--- source ---\n{}",
                    i, vm, js[k], a_src[i].1
                ));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    } else {
        eprintln!("node not found: skipping the VM/JavaScript comparison");
    }

    if !failures.is_empty() {
        let n = failures.len();
        failures.truncate(3);
        panic!("{} failure(s); the first:\n\n{}", n, failures.join("\n\n========\n\n"));
    }
    eprintln!(
        "{} of {} generated programs checked on every guarantee ({} returned a value, {} an error)",
        good.len(),
        cases,
        values,
        good.len() - values
    );
}
