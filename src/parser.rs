//! Recursive descent over the grammar in docs/SPEC.md §2.

use crate::ast::*;
use crate::diag::{Diagnostic, Severity, E0101_PARSE};
use crate::lexer::{lex, Tok, Token};

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    file: String,
}

pub fn parse_module(src: &str, path: &str) -> Result<Module, Diagnostic> {
    let (toks, comments) = lex(src, path).map_err(|(msg, span)| {
        Diagnostic::new(E0101_PARSE, Severity::Error, path, Some(span), msg)
    })?;
    let mut p = Parser {
        toks,
        pos: 0,
        file: path.to_string(),
    };
    let mut m = p.module(path)?;
    m.comments = comments;
    Ok(m)
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn span(&self) -> Span {
        self.toks[self.pos].span
    }
    fn peek2(&self) -> &Tok {
        let i = (self.pos + 1).min(self.toks.len() - 1);
        &self.toks[i].tok
    }
    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }
    fn err(&self, msg: &str) -> Diagnostic {
        Diagnostic::new(
            E0101_PARSE,
            Severity::Error,
            &self.file,
            Some(self.span()),
            msg.to_string(),
        )
    }
    fn expect(&mut self, t: Tok, what: &str) -> Result<Token, Diagnostic> {
        if *self.peek() == t {
            Ok(self.bump())
        } else {
            Err(self.err(&format!("expected {}", what)))
        }
    }
    fn ident(&mut self) -> Result<Ident, Diagnostic> {
        let sp = self.span();
        match self.peek().clone() {
            Tok::Ident(name) => {
                self.bump();
                Ok(Ident { name, span: sp })
            }
            _ => Err(self.err("expected identifier")),
        }
    }

    fn module(&mut self, path: &str) -> Result<Module, Diagnostic> {
        let mut imports = Vec::new();
        let mut fns = Vec::new();
        let mut tables = Vec::new();
        let mut externs = Vec::new();
        loop {
            match self.peek() {
                Tok::Eof => break,
                Tok::KwImport => imports.push(self.import()?),
                Tok::KwFn => fns.push(self.fndef(false)?),
                Tok::KwMemo => { self.bump(); fns.push(self.fndef(true)?); }
                Tok::KwProperty => { self.bump(); fns.push(self.fndef_opts(false, true)?); }
                Tok::KwTable => tables.push(self.tabledef()?),
                Tok::KwExtern => externs.push(self.externdef()?),
                _ => {
                    return Err(
                        self.err("expected `import`, `fn`, `table`, `extern`, or `memo` at top level")
                    )
                }
            }
        }
        Ok(Module {
            path: path.to_string(),
            imports,
            fns,
            tables,
            externs,
            comments: Vec::new(),
        })
    }

    /// extern fn name(params) [!cap...] = "host.path";
    fn externdef(&mut self) -> Result<ExternDef, Diagnostic> {
        let sp = self.span();
        self.expect(Tok::KwExtern, "`extern`")?;
        self.expect(Tok::KwFn, "`fn`")?;
        let name = self.ident()?;
        self.expect(Tok::LParen, "`(`")?;
        let mut params = Vec::new();
        if *self.peek() != Tok::RParen {
            params.push(self.ident()?);
            while *self.peek() == Tok::Comma {
                self.bump();
                params.push(self.ident()?);
            }
        }
        self.expect(Tok::RParen, "`)`")?;
        let caps = self.effect_list()?;
        self.expect(Tok::Assign, "`=`")?;
        let host = match self.bump().tok {
            Tok::Str(s) => s,
            _ => return Err(self.err("expected a host path string after `=`")),
        };
        self.expect(Tok::Semi, "`;`")?;
        Ok(ExternDef { name, params, caps, host, span: sp })
    }

    /// `xs: list` after a property's parameter — a generator directive, not
    /// a type. Refused anywhere else, so it can never be mistaken for one.
    fn gen_hint(&mut self, is_property: bool) -> Result<Option<String>, Diagnostic> {
        if *self.peek() != Tok::Colon {
            return Ok(None);
        }
        if !is_property {
            return Err(self.err(
                "a parameter hint is only legal on a `property` — rigid has no type annotations",
            ));
        }
        self.bump();
        // `int`, `float` and `str` are already builtin keywords, so the hint
        // is matched on the token rather than parsed as an identifier.
        let hint = match self.peek().clone() {
            Tok::KwInt => Some("int"),
            Tok::KwFloat => Some("float"),
            Tok::KwStr => Some("str"),
            Tok::Ident(s) if s == "list" => Some("list"),
            _ => None,
        };
        match hint {
            Some(h) => {
                self.bump();
                Ok(Some(h.to_string()))
            }
            None => Err(self.err("expected one of `int`, `float`, `str`, `list` after `:`")),
        }
    }

    /// `!io`, `!net !fs`, or nothing. Order is free; the set is canonical.
    fn effect_list(&mut self) -> Result<crate::ast::Caps, Diagnostic> {
        let mut caps: crate::ast::Caps = 0;
        while *self.peek() == Tok::Bang {
            self.bump();
            let k = self.ident()?;
            match crate::ast::cap_from_name(&k.name) {
                Some(c) => caps |= c,
                None => {
                    return Err(self.err(&format!(
                        "unknown effect `!{}` — expected one of {}",
                        k.name,
                        crate::ast::cap_vocabulary()
                    )))
                }
            }
        }
        Ok(caps)
    }

    fn import(&mut self) -> Result<Import, Diagnostic> {
        let sp = self.span();
        self.expect(Tok::KwImport, "`import`")?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut names = vec![self.ident()?];
        while *self.peek() == Tok::Comma {
            self.bump();
            names.push(self.ident()?);
        }
        self.expect(Tok::RBrace, "`}`")?;
        self.expect(Tok::KwFrom, "`from`")?;
        let from = match self.peek().clone() {
            Tok::Str(s) => {
                self.bump();
                s
            }
            _ => return Err(self.err("expected string module path")),
        };
        self.expect(Tok::Semi, "`;`")?;
        Ok(Import {
            names,
            from,
            span: sp,
        })
    }

    fn fndef(&mut self, memo: bool) -> Result<FnDef, Diagnostic> {
        self.fndef_opts(memo, false)
    }

    fn fndef_opts(&mut self, memo: bool, is_property: bool) -> Result<FnDef, Diagnostic> {
        // Only a property may carry generator hints, and they are parsed
        // inline with the parameter list below.
        let _ = is_property;
        let line = self.span().line;
        self.expect(Tok::KwFn, "`fn`")?;
        let name = self.ident()?;
        self.expect(Tok::LParen, "`(`")?;
        let mut params = Vec::new();
        let mut gen_hints: Vec<Option<String>> = Vec::new();
        if *self.peek() != Tok::RParen {
            params.push(self.ident()?);
            gen_hints.push(self.gen_hint(is_property)?);
            while *self.peek() == Tok::Comma {
                self.bump();
                params.push(self.ident()?);
                gen_hints.push(self.gen_hint(is_property)?);
            }
        }
        self.expect(Tok::RParen, "`)`")?;
        let caps = self.effect_list()?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut body = Vec::new();
        let mut stmt_lines = Vec::new();
        while *self.peek() != Tok::RBrace {
            stmt_lines.push(self.span().line);
            body.push(self.stmt()?);
        }
        self.expect(Tok::RBrace, "`}`")?;
        Ok(FnDef {
            memo,
            is_property,
            gen_hints,
            synthetic: false,
            captured: 0,
            name,
            params,
            caps,
            body,
            stmt_lines,
            line,
        })
    }

    fn tabledef(&mut self) -> Result<TableDef, Diagnostic> {
        let line = self.span().line;
        self.expect(Tok::KwTable, "`table`")?;
        let name = self.ident()?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut entries = Vec::new();
        let mut entry_lines = Vec::new();
        while *self.peek() != Tok::RBrace {
            let key_span = self.span();
            let key = match self.peek().clone() {
                Tok::Str(s) => {
                    self.bump();
                    s
                }
                _ => return Err(self.err("expected string key in table")),
            };
            self.expect(Tok::Arrow, "`->`")?;
            let target = self.ident()?;
            entry_lines.push(key_span.line);
            entries.push(TableEntry {
                key,
                key_span,
                target,
            });
            if *self.peek() == Tok::Comma {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(Tok::RBrace, "`}`")?;
        Ok(TableDef {
            name,
            entries,
            line,
            entry_lines,
        })
    }

    fn stmt(&mut self) -> Result<Stmt, Diagnostic> {
        if matches!(self.peek(), Tok::Ident(_)) && *self.peek2() == Tok::Assign {
            let name = self.ident()?;
            self.expect(Tok::Assign, "`=`")?;
            let value = self.expr()?;
            self.expect(Tok::Semi, "`;`")?;
            return Ok(Stmt::Assign { name, value });
        }
        if *self.peek() == Tok::KwWhile {
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let cond = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            self.expect(Tok::LBrace, "`{`")?;
            let mut body = Vec::new();
            while *self.peek() != Tok::RBrace {
                body.push(self.stmt()?);
            }
            self.expect(Tok::RBrace, "`}`")?;
            return Ok(Stmt::While { cond, body });
        }
        if *self.peek() == Tok::KwIf && self.starts_if_statement() {
            return self.if_statement();
        }
        if *self.peek() == Tok::KwFor {
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let var = self.ident()?;
            self.expect(Tok::KwIn, "`in`")?;
            let iter = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            let body = self.block()?;
            return Ok(Stmt::For { var, iter, body });
        }
        match self.peek() {
            Tok::KwLet => {
                self.bump();
                let name = self.ident()?;
                self.expect(Tok::Assign, "`=`")?;
                let value = self.expr()?;
                self.expect(Tok::Semi, "`;`")?;
                Ok(Stmt::Let { name, value })
            }
            Tok::KwReturn => {
                self.bump();
                let e = self.expr()?;
                self.expect(Tok::Semi, "`;`")?;
                Ok(Stmt::Return(e))
            }
            _ => {
                let e = self.expr()?;
                self.expect(Tok::Semi, "`;`")?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    fn block(&mut self) -> Result<Vec<Stmt>, Diagnostic> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut body = Vec::new();
        while *self.peek() != Tok::RBrace {
            body.push(self.stmt()?);
        }
        self.expect(Tok::RBrace, "`}`")?;
        Ok(body)
    }

    /// `if` is both an expression and a statement, so the two are told apart
    /// by what follows the condition: a `{` opens a statement block, anything
    /// else is the value-yielding form that has been here since v0.
    fn starts_if_statement(&self) -> bool {
        let mut i = self.pos + 1;
        if self.toks.get(i).map(|t| &t.tok) != Some(&Tok::LParen) {
            return false;
        }
        let mut depth = 0i32;
        while let Some(t) = self.toks.get(i) {
            match t.tok {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return self.toks.get(i + 1).map(|t| &t.tok) == Some(&Tok::LBrace);
                    }
                }
                Tok::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn if_statement(&mut self) -> Result<Stmt, Diagnostic> {
        self.expect(Tok::KwIf, "`if`")?;
        self.expect(Tok::LParen, "`(`")?;
        let cond = self.expr()?;
        self.expect(Tok::RParen, "`)`")?;
        let then = self.block()?;
        let mut els = Vec::new();
        if *self.peek() == Tok::KwElse {
            self.bump();
            if *self.peek() == Tok::KwIf {
                els.push(self.if_statement()?);
            } else {
                els = self.block()?;
            }
        }
        Ok(Stmt::IfStmt { cond, then, els })
    }

    fn expr(&mut self) -> Result<Expr, Diagnostic> {
        self.comparison()
    }

    fn comparison(&mut self) -> Result<Expr, Diagnostic> {
        let mut l = self.additive()?;
        loop {
            let op = match self.peek() {
                Tok::EqEq => BinOp::Eq,
                Tok::NotEq => BinOp::Ne,
                Tok::Lt => BinOp::Lt,
                Tok::Gt => BinOp::Gt,
                Tok::Le => BinOp::Le,
                Tok::Ge => BinOp::Ge,
                _ => break,
            };
            self.bump();
            let r = self.additive()?;
            l = Expr::Binop {
                op,
                l: Box::new(l),
                r: Box::new(r),
            };
        }
        Ok(l)
    }

    fn additive(&mut self) -> Result<Expr, Diagnostic> {
        let mut l = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let r = self.multiplicative()?;
            l = Expr::Binop {
                op,
                l: Box::new(l),
                r: Box::new(r),
            };
        }
        Ok(l)
    }

    fn multiplicative(&mut self) -> Result<Expr, Diagnostic> {
        let mut l = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Rem,
                _ => break,
            };
            self.bump();
            let r = self.unary()?;
            l = Expr::Binop {
                op,
                l: Box::new(l),
                r: Box::new(r),
            };
        }
        Ok(l)
    }

    fn unary(&mut self) -> Result<Expr, Diagnostic> {
        if *self.peek() == Tok::Minus {
            self.bump();
            let e = self.unary()?;
            return Ok(Expr::Neg(Box::new(e)));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, Diagnostic> {
        let mut e = self.atom()?;
        loop {
            match self.peek() {
                Tok::Dot => {
                    self.bump();
                    let field = self.ident()?;
                    e = Expr::Get {
                        base: Box::new(e),
                        field,
                    };
                }
                Tok::LBracket => {
                    self.bump();
                    let key = self.expr()?;
                    self.expect(Tok::RBracket, "`]`")?;
                    e = Expr::IndexE {
                        base: Box::new(e),
                        key: Box::new(key),
                    };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn atom(&mut self) -> Result<Expr, Diagnostic> {
        if *self.peek() == Tok::Amp {
            self.bump();
            let id = self.ident()?;
            return Ok(Expr::FunRef(id));
        }
        if *self.peek() == Tok::KwDyn {
            return self.dyncall();
        }
        if *self.peek() == Tok::KwPrint {
            let sp = self.span();
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let arg = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::Print {
                span: sp,
                arg: Box::new(arg),
            });
        }
        if *self.peek() == Tok::KwLen {
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let e = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::Len(Box::new(e)));
        }
        if *self.peek() == Tok::KwPush {
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let a = self.expr()?;
            self.expect(Tok::Comma, "`,`")?;
            let b = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::Push(Box::new(a), Box::new(b)));
        }
        if *self.peek() == Tok::KwStr {
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let e = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::StrConv(Box::new(e)));
        }
        if *self.peek() == Tok::KwInt {
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let e = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::IntConv(Box::new(e)));
        }
        if *self.peek() == Tok::KwFloat {
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let e = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::FloatConv(Box::new(e)));
        }
        if *self.peek() == Tok::KwSubstr {
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let a = self.expr()?;
            self.expect(Tok::Comma, "`,`")?;
            let b = self.expr()?;
            self.expect(Tok::Comma, "`,`")?;
            let c = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::Substr(Box::new(a), Box::new(b), Box::new(c)));
        }
        if *self.peek() == Tok::KwChars {
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let e = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::Chars(Box::new(e)));
        }
        if *self.peek() == Tok::KwJoin {
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let a = self.expr()?;
            self.expect(Tok::Comma, "`,`")?;
            let b = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::Join(Box::new(a), Box::new(b)));
        }
        if *self.peek() == Tok::KwReadFile {
            let sp = self.span();
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let e = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::ReadFile { span: sp, path: Box::new(e) });
        }
        if *self.peek() == Tok::KwWriteFile {
            let sp = self.span();
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let a = self.expr()?;
            self.expect(Tok::Comma, "`,`")?;
            let b = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::WriteFile { span: sp, path: Box::new(a), contents: Box::new(b) });
        }
        if *self.peek() == Tok::KwFail {
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let e = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::Fail(Box::new(e)));
        }
        if *self.peek() == Tok::KwTry {
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            let e = self.expr()?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::Try(Box::new(e)));
        }
        if *self.peek() == Tok::KwInput {
            let sp = self.span();
            self.bump();
            self.expect(Tok::LParen, "`(`")?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(Expr::Input { span: sp });
        }
        self.call()
    }

    fn dyncall(&mut self) -> Result<Expr, Diagnostic> {
        self.expect(Tok::KwDyn, "`dyn`")?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut targets = vec![self.ident()?];
        while *self.peek() == Tok::Comma {
            self.bump();
            targets.push(self.ident()?);
        }
        self.expect(Tok::RBrace, "`}`")?;
        self.expect(Tok::LParen, "`(`")?;
        let callee = self.expr()?;
        let mut args = Vec::new();
        while *self.peek() == Tok::Comma {
            self.bump();
            args.push(self.expr()?);
        }
        self.expect(Tok::RParen, "`)`")?;
        Ok(Expr::DynCall {
            targets,
            callee: Box::new(callee),
            args,
        })
    }

    fn call(&mut self) -> Result<Expr, Diagnostic> {
        // v0: only direct calls of named definitions: ident '(' args ')'
        let sp = self.span();
        match self.peek().clone() {
            Tok::At => {
                let sp = self.span();
                self.bump();
                let h = match self.bump().tok {
                    Tok::Str(s) => s,
                    _ => return Err(self.err("expected a hash string after `@`")),
                };
                self.expect(Tok::LParen, "`(`")?;
                let mut args = Vec::new();
                if *self.peek() != Tok::RParen {
                    args.push(self.expr()?);
                    while *self.peek() == Tok::Comma {
                        self.bump();
                        args.push(self.expr()?);
                    }
                }
                self.expect(Tok::RParen, "`)`")?;
                return Ok(Expr::TimeCall { hash: h, args, span: sp });
            }

            Tok::Ident(name) => {
                self.bump();
                if *self.peek() == Tok::LBracket {
                    self.bump();
                    let key = self.expr()?;
                    self.expect(Tok::RBracket, "`]`")?;
                    let args = if *self.peek() == Tok::LParen {
                        self.bump();
                        let mut a = Vec::new();
                        if *self.peek() != Tok::RParen {
                            a.push(self.expr()?);
                            while *self.peek() == Tok::Comma {
                                self.bump();
                                a.push(self.expr()?);
                            }
                        }
                        self.expect(Tok::RParen, "`)`")?;
                        Some(a)
                    } else {
                        None
                    };
                    return Ok(Expr::TableDispatch {
                        table: Ident { name, span: sp },
                        key: Box::new(key),
                        args,
                    });
                }
                if *self.peek() == Tok::LParen {
                    self.bump();
                    let mut args = Vec::new();
                    if *self.peek() != Tok::RParen {
                        args.push(self.expr()?);
                        while *self.peek() == Tok::Comma {
                            self.bump();
                            args.push(self.expr()?);
                        }
                    }
                    self.expect(Tok::RParen, "`)`")?;
                    Ok(Expr::Call {
                        callee: Ident { name, span: sp },
                        args,
                    })
                } else {
                    Ok(Expr::Var(Ident { name, span: sp }))
                }
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<Expr, Diagnostic> {
        match self.peek().clone() {
            Tok::Int(v) => {
                self.bump();
                Ok(Expr::Int(v))
            }
            Tok::Float(v) => {
                self.bump();
                Ok(Expr::Float(v))
            }
            Tok::KwFn => {
                let line = self.span().line;
                self.bump();
                self.expect(Tok::LParen, "`(`")?;
                let mut params = Vec::new();
                if *self.peek() != Tok::RParen {
                    params.push(self.ident()?);
                    while *self.peek() == Tok::Comma {
                        self.bump();
                        params.push(self.ident()?);
                    }
                }
                self.expect(Tok::RParen, "`)`")?;
                let body = self.block()?;
                Ok(Expr::Lambda {
                    params,
                    body,
                    lifted: None,
                    captures: Vec::new(),
                    line,
                })
            }
            Tok::Str(s) => {
                self.bump();
                Ok(Expr::Str(s))
            }
            Tok::LParen => {
                self.bump();
                let e = self.expr()?;
                self.expect(Tok::RParen, "`)`")?;
                Ok(e)
            }
            Tok::LBracket => {
                self.bump();
                let mut elems = Vec::new();
                if *self.peek() != Tok::RBracket {
                    elems.push(self.expr()?);
                    while *self.peek() == Tok::Comma {
                        self.bump();
                        if *self.peek() == Tok::RBracket {
                            break;
                        }
                        elems.push(self.expr()?);
                    }
                }
                self.expect(Tok::RBracket, "`]`")?;
                Ok(Expr::ListLit(elems))
            }
            Tok::LBrace => {
                self.bump();
                let mut fields = Vec::new();
                if *self.peek() != Tok::RBrace {
                    loop {
                        let name = self.ident()?;
                        self.expect(Tok::Colon, "`:`")?;
                        let v = self.expr()?;
                        fields.push((name, v));
                        if *self.peek() == Tok::Comma {
                            self.bump();
                            if *self.peek() == Tok::RBrace {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
                self.expect(Tok::RBrace, "`}`")?;
                Ok(Expr::RecordLit(fields))
            }
            Tok::KwIf => {
                self.bump();
                self.expect(Tok::LParen, "`(`")?;
                let c = self.expr()?;
                self.expect(Tok::RParen, "`)`")?;
                let t = self.expr()?;
                self.expect(Tok::KwElse, "`else`")?;
                let e = self.expr()?;
                Ok(Expr::If {
                    c: Box::new(c),
                    t: Box::new(t),
                    e: Box::new(e),
                })
            }
            _ => Err(self.err("expected expression")),
        }
    }
}
