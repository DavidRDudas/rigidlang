//! Canonical form and hashing. docs/SPEC.md §3.
//!
//! Identity is the SHA-256 of a definition's canonical bytes, in which:
//!   - local names are erased (binding-order slot indices),
//!   - references to other definitions are their 32-byte hashes,
//!   - self-reference is a fixed SELF tag,
//!   - references inside a mutual-recursion component are CREF(ordinal),
//!     and every member's identity derives from the component hash (§3.4).

use crate::ast::*;
use crate::resolve::{Binding, DefId, Workspace};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub type Hash = [u8; 32];

pub fn hex(h: &Hash) -> String {
    let mut s = String::with_capacity(71);
    s.push_str("sha256:");
    for b in h {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// canonical tags — frozen; append only (docs/SPEC.md §3.2)
const T_INT: u8 = 0x01;
const T_STR: u8 = 0x02;
const T_LOCAL: u8 = 0x03;
const T_REF: u8 = 0x04;
const T_SELF: u8 = 0x05;
const T_CALL: u8 = 0x06;
const T_BINOP: u8 = 0x07;
const T_IF: u8 = 0x08;
const T_NEG: u8 = 0x09;
const T_CREF: u8 = 0x0A; // intra-component reference, u32 ordinal
const T_FUNREF: u8 = 0x0B; // first-class reference to a definition
const T_DYN: u8 = 0x0C; // quarantined indirect call: set + callee + args
const T_PRINT: u8 = 0x0D; // io builtin
const T_LET: u8 = 0x10;
const T_RETURN: u8 = 0x11;
const T_EXPRSTMT: u8 = 0x12;
const T_FN: u8 = 0x20;
const T_SCC: u8 = 0x21; // component header: u32 member count + members
const T_MEMBER: u8 = 0x22; // member identity derivation: component hash + ordinal
const T_FNIO: u8 = 0x23; // fn header for `!io` definitions (T_FN stays pure-only)
const T_TABLE: u8 = 0x24; // table def: u32 count + key-sorted entries
const T_MODULE: u8 = 0x2A;
const T_EXTERN: u8 = 0x2B;
const T_CLOSURE: u8 = 0x32; // lifted-def ref + u32 capture count + slots
const T_IFSTMT: u8 = 0x30; // cond + u32 then-count + then + u32 else-count + else
const T_FOR: u8 = 0x31; // u32 loop slot + iterable + u32 stmt count + stmts
const T_FLOAT: u8 = 0x2E; // f64, canonicalized (see `canon_f64`)
const T_FLOATC: u8 = 0x2F; // float(e)
const T_FNEFF: u8 = 0x2D; // fn header carrying an explicit capability mask
const T_TIMECALL: u8 = 0x2C; // hash string + args // extern decl: u32 arity + u8 io + u32 hostlen + host // module identity: u32 count + sorted member hashes
const T_TDISPATCH: u8 = 0x0E; // dispatch site: table ref + key + args
const T_LIDX: u8 = 0x0F; // value index rooted at a local: slot + key
const T_LIST: u8 = 0x13; // u32 count + elements
const T_LEN: u8 = 0x14;
const T_PUSH: u8 = 0x15;
const T_RECORD: u8 = 0x16; // u32 count + name-sorted (u32 len + name + value)
const T_GET: u8 = 0x17; // base + u32 len + field name
const T_IDXE: u8 = 0x18; // generic value index: base + key
const T_INPUT: u8 = 0x19;
const T_ASSIGN: u8 = 0x1A; // u32 slot + value
const T_WHILE: u8 = 0x1B; // cond + u32 stmt count + stmts
const T_FAIL: u8 = 0x1C;
const T_TRY: u8 = 0x1D;
const T_STRC: u8 = 0x1E;
const T_INTC: u8 = 0x1F;
const T_SUBSTR: u8 = 0x25;
const T_CHARS: u8 = 0x26;
const T_JOIN: u8 = 0x27;
const T_READF: u8 = 0x28;
const T_WRITEF: u8 = 0x29;

/// The canonical bits of an f64.
///
/// IEEE-754 has values that are equal but differently encoded (`-0.0` and
/// `+0.0`) and a value unequal to itself (NaN). Content addressing cannot
/// tolerate either: two things that compare equal must key identically, and
/// a thing must key the same as itself. So `-0.0` normalizes to `+0.0` and
/// every NaN to one quiet NaN; every other value keeps its own bits.
///
/// Source cannot spell either one — `-0.0` parses as a negation of `0.0`,
/// and there is no NaN literal — so in the *hash* path this is a rule
/// waiting for a case. It earns its place in the `memo` key path, where both
/// values arise from ordinary arithmetic and a cache that missed on
/// `0.0 * -1.0` would be silently wrong.
pub fn canon_f64(v: f64) -> u64 {
    if v.is_nan() {
        0x7ff8_0000_0000_0000
    } else if v == 0.0 {
        0
    } else {
        v.to_bits()
    }
}

const CREF_PLACEHOLDER: u32 = 0xFFFF_FFFF;

struct Enc<'a> {
    ws: &'a Workspace,
    hashes: &'a HashMap<DefId, Hash>,
    def: DefId,
    /// members of this def's SCC (multi-member cycles only) -> ordinal
    scc: &'a HashMap<DefId, u32>,
    placeholder: bool,
    out: Vec<u8>,
}

impl<'a> Enc<'a> {
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }

    fn binding(&self, span: Span, name: &str) -> Binding {
        *self
            .ws
            .bindings
            .get(&(self.def, span.line, span.col, name.to_string()))
            .unwrap_or(&Binding::Broken)
    }

    fn name_ref(&mut self, span: Span, name: &str) {
        match self.binding(span, name) {
            Binding::Local(slot) => {
                self.out.push(T_LOCAL);
                self.u32(slot);
            }
            Binding::Def(d) if self.scc.contains_key(&d) => {
                self.out.push(T_CREF);
                let ord = if self.placeholder {
                    CREF_PLACEHOLDER
                } else {
                    self.scc[&d]
                };
                self.u32(ord);
            }
            Binding::Def(d) if d == self.def => self.out.push(T_SELF),
            Binding::Def(d) => {
                self.out.push(T_REF);
                let h = self.hashes.get(&d).expect("hash order violated");
                self.out.extend_from_slice(h);
            }
            Binding::Broken => unreachable!("poisoned defs are never encoded"),
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Int(v) => {
                self.out.push(T_INT);
                self.i64(*v);
            }
            Expr::Float(v) => {
                self.out.push(T_FLOAT);
                self.out.extend_from_slice(&canon_f64(*v).to_be_bytes());
            }
            Expr::Str(s) => {
                self.out.push(T_STR);
                self.u32(s.len() as u32);
                self.out.extend_from_slice(s.as_bytes());
            }
            Expr::Var(id) => self.name_ref(id.span, &id.name),
            Expr::TimeCall { hash, args, .. } => {
                self.out.push(T_TIMECALL);
                self.u32(hash.len() as u32);
                self.out.extend_from_slice(hash.as_bytes());
                self.u32(args.len() as u32);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Call { callee, args } => {
                self.out.push(T_CALL);
                self.name_ref(callee.span, &callee.name);
                self.u32(args.len() as u32);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Binop { op, l, r } => {
                self.out.push(T_BINOP);
                self.out.push(op.tag());
                self.expr(l);
                self.expr(r);
            }
            Expr::If { c, t, e } => {
                self.out.push(T_IF);
                self.expr(c);
                self.expr(t);
                self.expr(e);
            }
            Expr::Neg(x) => {
                self.out.push(T_NEG);
                self.expr(x);
            }
            Expr::FunRef(id) => {
                self.out.push(T_FUNREF);
                self.name_ref(id.span, &id.name);
            }
            Expr::DynCall {
                targets,
                callee,
                args,
            } => {
                self.out.push(T_DYN);
                // canonical set order: sort by encoded reference bytes (name-free)
                let mut encoded: Vec<Vec<u8>> = targets
                    .iter()
                    .map(|t| {
                        let mut sub = Enc {
                            ws: self.ws,
                            hashes: self.hashes,
                            def: self.def,
                            scc: self.scc,
                            placeholder: self.placeholder,
                            out: Vec::new(),
                        };
                        sub.name_ref(t.span, &t.name);
                        sub.out
                    })
                    .collect();
                encoded.sort();
                self.u32(encoded.len() as u32);
                for e in encoded {
                    self.out.extend(e);
                }
                self.expr(callee);
                self.u32(args.len() as u32);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Print { arg, .. } => {
                self.out.push(T_PRINT);
                self.expr(arg);
            }
            Expr::TableDispatch { table, key, args } => {
                // resolution-directed: a local base is value indexing
                if let Binding::Local(slot) = self.binding(table.span, &table.name) {
                    self.out.push(T_LIDX);
                    self.u32(slot);
                    self.expr(key);
                    return;
                }
                self.out.push(T_TDISPATCH);
                self.name_ref(table.span, &table.name);
                self.expr(key);
                match args {
                    None => self.out.push(0),
                    Some(a) => {
                        self.out.push(1);
                        self.u32(a.len() as u32);
                        for x in a {
                            self.expr(x);
                        }
                    }
                }
            }
            Expr::ListLit(elems) => {
                self.out.push(T_LIST);
                self.u32(elems.len() as u32);
                for x in elems {
                    self.expr(x);
                }
            }
            Expr::RecordLit(fields) => {
                self.out.push(T_RECORD);
                self.u32(fields.len() as u32);
                let mut fs: Vec<(&String, &Expr)> =
                    fields.iter().map(|(n, v)| (&n.name, v)).collect();
                fs.sort_by(|a, b| a.0.cmp(b.0));
                for (n, v) in fs {
                    self.u32(n.len() as u32);
                    self.out.extend_from_slice(n.as_bytes());
                    self.expr(v);
                }
            }
            Expr::Get { base, field } => {
                self.out.push(T_GET);
                self.expr(base);
                self.u32(field.name.len() as u32);
                self.out.extend_from_slice(field.name.as_bytes());
            }
            Expr::IndexE { base, key } => {
                self.out.push(T_IDXE);
                self.expr(base);
                self.expr(key);
            }
            Expr::Len(x) => {
                self.out.push(T_LEN);
                self.expr(x);
            }
            Expr::Push(a, b) => {
                self.out.push(T_PUSH);
                self.expr(a);
                self.expr(b);
            }
            Expr::Input { .. } => {
                self.out.push(T_INPUT);
            }
            Expr::Fail(x) => {
                self.out.push(T_FAIL);
                self.expr(x);
            }
            Expr::Try(x) => {
                self.out.push(T_TRY);
                self.expr(x);
            }
            Expr::StrConv(x) => {
                self.out.push(T_STRC);
                self.expr(x);
            }
            Expr::FloatConv(x) => {
                self.out.push(T_FLOATC);
                self.expr(x);
            }
            Expr::Lambda {
                lifted, captures, ..
            } => {
                // The body is not encoded here — it belongs to the lifted
                // definition, and that definition's hash stands in for it.
                // So renaming a captured local moves no hash, and editing
                // the lambda's body moves exactly one.
                self.out.push(T_CLOSURE);
                match lifted {
                    Some(id) => self.name_ref(id.span, &id.name),
                    None => self.out.push(0),
                }
                self.u32(captures.len() as u32);
                for c in captures {
                    match self.binding(c.span, &c.name) {
                        crate::resolve::Binding::Local(slot) => self.u32(slot),
                        _ => self.u32(u32::MAX),
                    }
                }
            }
            Expr::IntConv(x) => {
                self.out.push(T_INTC);
                self.expr(x);
            }
            Expr::Substr(a, b, x) => {
                self.out.push(T_SUBSTR);
                self.expr(a);
                self.expr(b);
                self.expr(x);
            }
            Expr::Chars(x) => {
                self.out.push(T_CHARS);
                self.expr(x);
            }
            Expr::Join(a, b) => {
                self.out.push(T_JOIN);
                self.expr(a);
                self.expr(b);
            }
            Expr::ReadFile { path, .. } => {
                self.out.push(T_READF);
                self.expr(path);
            }
            Expr::WriteFile { path, contents, .. } => {
                self.out.push(T_WRITEF);
                self.expr(path);
                self.expr(contents);
            }
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { value, .. } => {
                self.out.push(T_LET);
                self.expr(value);
            }
            Stmt::Assign { name, value } => {
                self.out.push(T_ASSIGN);
                match self.binding(name.span, &name.name) {
                    Binding::Local(slot) => self.u32(slot),
                    _ => unreachable!("assign to non-local survived resolution"),
                }
                self.expr(value);
            }
            Stmt::While { cond, body } => {
                self.out.push(T_WHILE);
                self.expr(cond);
                self.u32(body.len() as u32);
                for bs in body {
                    self.stmt(bs);
                }
            }
            Stmt::IfStmt { cond, then, els } => {
                self.out.push(T_IFSTMT);
                self.expr(cond);
                self.u32(then.len() as u32);
                for s in then {
                    self.stmt(s);
                }
                self.u32(els.len() as u32);
                for s in els {
                    self.stmt(s);
                }
            }
            Stmt::For { var, iter, body } => {
                self.out.push(T_FOR);
                // The loop variable is a bound name, so only its slot is
                // canonical — renaming it must not move the hash.
                match self.binding(var.span, &var.name) {
                    crate::resolve::Binding::Local(slot) => self.u32(slot),
                    _ => self.u32(u32::MAX),
                }
                self.expr(iter);
                self.u32(body.len() as u32);
                for s in body {
                    self.stmt(s);
                }
            }
            Stmt::Return(e) => {
                self.out.push(T_RETURN);
                self.expr(e);
            }
            Stmt::Expr(e) => {
                self.out.push(T_EXPRSTMT);
                self.expr(e);
            }
        }
    }
}

fn encode_one(
    ws: &Workspace,
    hashes: &HashMap<DefId, Hash>,
    def: DefId,
    scc: &HashMap<DefId, u32>,
    placeholder: bool,
) -> Vec<u8> {
    let info = &ws.defs[def];
    assert!(matches!(ws.defs[def].kind, crate::resolve::DefKind::Fn));
    let f = &ws.modules[info.module_idx].fns[info.item_idx];
    let mut enc = Enc {
        ws,
        hashes,
        def,
        scc,
        placeholder,
        out: Vec::new(),
    };
    // Tags are append-only, so the two sets that existed before capabilities
    // keep their original encoding and their original hashes. Anything finer
    // gets the new tag plus an explicit mask.
    match f.caps {
        0 => enc.out.push(T_FN),
        crate::ast::CAP_IO => enc.out.push(T_FNIO),
        c => {
            enc.out.push(T_FNEFF);
            enc.u32(c as u32);
        }
    }
    enc.u32(f.params.len() as u32);
    enc.u32(f.body.len() as u32);
    for s in &f.body {
        enc.stmt(s);
    }
    enc.out
}

/// Canonical bytes for any definition kind.
fn def_bytes(
    ws: &Workspace,
    hashes: &HashMap<DefId, Hash>,
    d: DefId,
    scc: &HashMap<DefId, u32>,
    placeholder: bool,
) -> Vec<u8> {
    match ws.defs[d].kind {
        crate::resolve::DefKind::Fn => encode_one(ws, hashes, d, scc, placeholder),
        crate::resolve::DefKind::Extern => {
            let info = &ws.defs[d];
            let x = &ws.modules[info.module_idx].externs[info.item_idx];
            let mut out = vec![T_EXTERN];
            out.extend_from_slice(&(x.params.len() as u32).to_be_bytes());
            // 0 and 1 are the pre-capability encodings; 2 escapes to a mask.
            match x.caps {
                0 => out.push(0),
                crate::ast::CAP_IO => out.push(1),
                c => {
                    out.push(2);
                    out.extend_from_slice(&(c as u32).to_be_bytes());
                }
            }
            out.extend_from_slice(&(x.host.len() as u32).to_be_bytes());
            out.extend_from_slice(x.host.as_bytes());
            out
        }
        crate::resolve::DefKind::Table => {
            // T_TABLE + u32 count + entries sorted by key bytes:
            // (u32 keylen + key + target ref-encoding)
            let mut entries: Vec<(&String, DefId)> = ws
                .table_entries
                .get(&d)
                .map(|v| v.iter().map(|(k, t)| (k, *t)).collect())
                .unwrap_or_default();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let mut out = vec![T_TABLE];
            out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
            for (k, t) in entries {
                out.extend_from_slice(&(k.len() as u32).to_be_bytes());
                out.extend_from_slice(k.as_bytes());
                if let Some(&ord) = scc.get(&t) {
                    out.push(T_CREF);
                    let o = if placeholder { CREF_PLACEHOLDER } else { ord };
                    out.extend_from_slice(&o.to_be_bytes());
                } else {
                    out.push(T_REF);
                    out.extend_from_slice(hashes.get(&t).expect("hash order violated"));
                }
            }
            out
        }
    }
}

/// Module identity: hash over the sorted hashes of the module's members.
/// Names excluded — renaming or reordering definitions leaves it fixed.
pub fn module_hash(ws: &Workspace, hashes: &HashMap<DefId, Hash>, path: &str) -> Option<Hash> {
    let mut members: Vec<Hash> = ws
        .defs
        .iter()
        .enumerate()
        .filter(|(_, i)| i.module_path == path)
        .filter_map(|(d, _)| hashes.get(&d).copied())
        .collect();
    let count = ws.defs.iter().filter(|i| i.module_path == path).count();
    if members.len() != count {
        return None; // some member unhashed (poisoned)
    }
    members.sort();
    let mut out = vec![T_MODULE];
    out.extend_from_slice(&(members.len() as u32).to_be_bytes());
    for h in members {
        out.extend_from_slice(&h);
    }
    Some(sha(&out))
}

pub fn module_hash_hex(ws: &Workspace, hashes: &HashMap<DefId, Hash>, path: &str) -> String {
    module_hash(ws, hashes, path)
        .map(|h| hex(&h))
        .unwrap_or_default()
}

fn sha(bytes: &[u8]) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut h: Hash = [0; 32];
    h.copy_from_slice(&out);
    h
}

/// Hash every non-poisoned definition, SCC by SCC in dependency order.
pub fn hash_workspace(ws: &Workspace) -> HashMap<DefId, Hash> {
    let mut hashes: HashMap<DefId, Hash> = HashMap::new();
    for members in &ws.sccs {
        if members.iter().any(|&d| ws.defs[d].poisoned) {
            continue; // cannot hash a component with a poisoned member
        }
        if members
            .iter()
            .any(|&d| ws.edges[d].iter().any(|&t| !members.contains(&t) && !hashes.contains_key(&t)))
        {
            continue; // an external dep was skipped (poisoned upstream)
        }
        if members.len() == 1 {
            let d = members[0];
            let bytes = def_bytes(ws, &hashes, d, &HashMap::new(), false);
            hashes.insert(d, sha(&bytes));
        } else {
            // §3.4: order members by their placeholder encodings, name-free;
            // tie-break by (module, name) — a documented pragmatic leak that
            // only bites structurally identical cycle twins.
            let scc_set: HashMap<DefId, u32> =
                members.iter().map(|&d| (d, CREF_PLACEHOLDER)).collect();
            let mut keyed: Vec<(Vec<u8>, String, String, DefId)> = members
                .iter()
                .map(|&d| {
                    (
                        def_bytes(ws, &hashes, d, &scc_set, true),
                        ws.defs[d].module_path.clone(),
                        ws.defs[d].name.clone(),
                        d,
                    )
                })
                .collect();
            keyed.sort();
            let ordinals: HashMap<DefId, u32> = keyed
                .iter()
                .enumerate()
                .map(|(i, (_, _, _, d))| (*d, i as u32))
                .collect();
            let mut comp_bytes = vec![T_SCC];
            comp_bytes.extend_from_slice(&(keyed.len() as u32).to_be_bytes());
            for (_, _, _, d) in &keyed {
                comp_bytes.extend(def_bytes(ws, &hashes, *d, &ordinals, false));
            }
            let comp_hash = sha(&comp_bytes);
            for (_, _, _, d) in &keyed {
                let mut mb = vec![T_MEMBER];
                mb.extend_from_slice(&comp_hash);
                mb.extend_from_slice(&ordinals[d].to_be_bytes());
                hashes.insert(*d, sha(&mb));
            }
        }
    }
    hashes
}
