//! Structured diagnostics. Stable codes, machine-consumable JSON, typed fix
//! hints. Agents are first-class readers of compiler output. docs/SPEC.md §5.

use crate::ast::Span;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

// Stable codes. Never renumber; only append.
pub const E0101_PARSE: &str = "E0101"; // syntax error
pub const E0201_UNRESOLVED: &str = "E0201"; // name does not resolve
pub const E0202_RESOLUTION_CHANGED: &str = "E0202"; // pinned target now bound to a different definition
pub const E0203_AMBIGUOUS: &str = "E0203"; // local definition collides with imported name
pub const E0204_DYN_REQUIRED: &str = "E0204"; // calling a value outside a dyn block
pub const E0205_BAD_FUNREF: &str = "E0205"; // `&` of something that is not a definition
pub const E0206_DEF_AS_VALUE: &str = "E0206"; // bare definition in value position; use `&name`
pub const E0207_EFFECT: &str = "E0207"; // io effect used or reached without `!io`
pub const E0208_TABLE: &str = "E0208"; // table misuse (direct call, non-table dispatch, bad target)
pub const E0209_ARITY: &str = "E0209"; // argument count does not match the target's arity
pub const E0210_ASSIGN: &str = "E0210"; // assignment target is not a local
pub const W0403_NARROW_DYN: &str = "W0403"; // a funref may reach a dyn site outside its declared set
pub const W0404_QUARANTINE_DRIFT: &str = "W0404"; // pinned dyn/table target content changed
pub const I0405_INFERRED_SET: &str = "I0405";
pub const E0211_MEMO_IO: &str = "E0211"; // memo fn reaches !io — memoization requires purity
pub const E0214_RULE: &str = "E0214"; // a declared relationship in rigid.rules is violated
pub const E0213_TARGET: &str = "E0213"; // the selected compile target has no representation for this construct
pub const E0212_DOMAIN: &str = "E0212"; // literal argument outside a parameter's inferred value domain
pub const I0407_UNTESTED_TABLE: &str = "I0407"; // a dispatch table no test reaches
pub const I0408_KEY_SHADOWS: &str = "I0408"; // a key names another target of the same table
pub const I0406_LOCAL_RENAME: &str = "I0406"; // a same-module def was renamed; broken local refs are repairable // inferred quarantine set for an unquarantined call
#[allow(dead_code)] // retired in v1 (SCC hashing); code stays reserved, never reuse
pub const E0301_CYCLE: &str = "E0301";
pub const W0405_CAP_WIDENED: &str = "W0405"; // an imported definition gained a capability it did not have when pinned
pub const W0401_DRIFT: &str = "W0401"; // pinned target's content changed; repinned (error under --frozen)
pub const I0402_PINNED: &str = "I0402"; // new reference pinned

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub file: String,
    pub span: Option<Span>,
    pub message: String,
    /// Machine-readable payload: hashes, candidate names, targets.
    pub data: Value,
    /// Typed fix hints an agent can apply mechanically.
    pub fixes: Vec<Value>,
}

impl Diagnostic {
    pub fn new(
        code: &'static str,
        severity: Severity,
        file: &str,
        span: Option<Span>,
        message: String,
    ) -> Self {
        Diagnostic {
            code,
            severity,
            file: file.to_string(),
            span,
            message,
            data: Value::Null,
            fixes: Vec::new(),
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    pub fn with_fix(mut self, fix: Value) -> Self {
        self.fixes.push(fix);
        self
    }

    pub fn to_json(&self) -> Value {
        json!({
            "code": self.code,
            "severity": self.severity.as_str(),
            "file": self.file,
            "span": self.span.map(|s| json!({"line": s.line, "col": s.col})),
            "message": self.message,
            "data": self.data,
            "fixes": self.fixes,
        })
    }

    pub fn render_human(&self) -> String {
        let loc = match self.span {
            Some(s) => format!("{}:{}:{}", self.file, s.line, s.col),
            None => self.file.clone(),
        };
        format!(
            "{}[{}] {}: {}",
            self.severity.as_str(),
            self.code,
            loc,
            self.message
        )
    }
}

pub fn emit(diags: &[Diagnostic], json_mode: bool) {
    if json_mode {
        let arr: Vec<Value> = diags.iter().map(|d| d.to_json()).collect();
        println!("{}", serde_json::to_string_pretty(&arr).unwrap());
    } else {
        for d in diags {
            println!("{}", d.render_human());
        }
    }
}

pub fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| d.severity == Severity::Error)
}
