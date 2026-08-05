//! Hand-rolled lexer. Tracks line/col for spans. `//` comments to EOL.

use crate::ast::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),
    KwFn,
    KwLet,
    KwReturn,
    KwImport,
    KwFrom,
    KwIf,
    KwElse,
    KwDyn,
    KwPrint,
    KwTable,
    KwExtern,
    KwMemo,
    At,
    KwLen,
    KwPush,
    KwInput,
    KwWhile,
    KwStr,
    KwInt,
    KwFloat,
    KwIn,
    KwProperty,
    KwFor,
    KwSubstr,
    KwChars,
    KwJoin,
    KwReadFile,
    KwWriteFile,
    KwFail,
    KwTry,
    Dot,
    Colon,
    LBracket,
    RBracket,
    Arrow,
    Amp,
    Bang,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Assign,
    EqEq,
    NotEq,
    Lt,
    Gt,
    Le,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

pub fn lex(
    src: &str,
    file: &str,
) -> Result<(Vec<Token>, Vec<crate::ast::Comment>), (String, Span)> {
    let mut out = Vec::new();
    let mut comments: Vec<crate::ast::Comment> = Vec::new();
    let bytes: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;
    let n = bytes.len();

    macro_rules! push {
        ($t:expr, $l:expr, $c:expr) => {
            out.push(Token {
                tok: $t,
                span: Span { line: $l, col: $c },
            })
        };
    }

    while i < n {
        let c = bytes[i];
        let (sl, sc) = (line, col);
        match c {
            ' ' | '\t' | '\r' => {
                i += 1;
                col += 1;
            }
            '\n' => {
                i += 1;
                line += 1;
                col = 1;
            }
            '/' if i + 1 < n && bytes[i + 1] == '/' => {
                let cl = line;
                i += 2;
                let mut text = String::new();
                while i < n && bytes[i] != '\n' {
                    text.push(bytes[i]);
                    i += 1;
                }
                comments.push(crate::ast::Comment {
                    line: cl,
                    text: text.trim().to_string(),
                });
            }
            '(' => {
                push!(Tok::LParen, sl, sc);
                i += 1;
                col += 1;
            }
            ')' => {
                push!(Tok::RParen, sl, sc);
                i += 1;
                col += 1;
            }
            '[' => {
                push!(Tok::LBracket, sl, sc);
                i += 1;
                col += 1;
            }
            ']' => {
                push!(Tok::RBracket, sl, sc);
                i += 1;
                col += 1;
            }
            '{' => {
                push!(Tok::LBrace, sl, sc);
                i += 1;
                col += 1;
            }
            '}' => {
                push!(Tok::RBrace, sl, sc);
                i += 1;
                col += 1;
            }
            '.' => {
                push!(Tok::Dot, sl, sc);
                i += 1;
                col += 1;
            }
            ':' => {
                push!(Tok::Colon, sl, sc);
                i += 1;
                col += 1;
            }
            ',' => {
                push!(Tok::Comma, sl, sc);
                i += 1;
                col += 1;
            }
            ';' => {
                push!(Tok::Semi, sl, sc);
                i += 1;
                col += 1;
            }
            '@' => {
                push!(Tok::At, sl, sc);
                i += 1;
                col += 1;
            }
            '+' => {
                push!(Tok::Plus, sl, sc);
                i += 1;
                col += 1;
            }
            '-' => {
                if i + 1 < n && bytes[i + 1] == '>' {
                    push!(Tok::Arrow, sl, sc);
                    i += 2;
                    col += 2;
                } else {
                    push!(Tok::Minus, sl, sc);
                    i += 1;
                    col += 1;
                }
            }
            '*' => {
                push!(Tok::Star, sl, sc);
                i += 1;
                col += 1;
            }
            '/' => {
                push!(Tok::Slash, sl, sc);
                i += 1;
                col += 1;
            }
            '%' => {
                push!(Tok::Percent, sl, sc);
                i += 1;
                col += 1;
            }
            '=' => {
                if i + 1 < n && bytes[i + 1] == '=' {
                    push!(Tok::EqEq, sl, sc);
                    i += 2;
                    col += 2;
                } else {
                    push!(Tok::Assign, sl, sc);
                    i += 1;
                    col += 1;
                }
            }
            '!' => {
                if i + 1 < n && bytes[i + 1] == '=' {
                    push!(Tok::NotEq, sl, sc);
                    i += 2;
                    col += 2;
                } else {
                    push!(Tok::Bang, sl, sc);
                    i += 1;
                    col += 1;
                }
            }
            '&' => {
                push!(Tok::Amp, sl, sc);
                i += 1;
                col += 1;
            }
            '<' => {
                if i + 1 < n && bytes[i + 1] == '=' {
                    push!(Tok::Le, sl, sc);
                    i += 2;
                    col += 2;
                } else {
                    push!(Tok::Lt, sl, sc);
                    i += 1;
                    col += 1;
                }
            }
            '>' => {
                if i + 1 < n && bytes[i + 1] == '=' {
                    push!(Tok::Ge, sl, sc);
                    i += 2;
                    col += 2;
                } else {
                    push!(Tok::Gt, sl, sc);
                    i += 1;
                    col += 1;
                }
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                col += 1;
                while i < n && bytes[i] != '"' {
                    if bytes[i] == '\n' {
                        return Err((
                            format!("{}: unterminated string", file),
                            Span { line: sl, col: sc },
                        ));
                    }
                    if bytes[i] == '\\' {
                        if i + 1 >= n {
                            return Err((
                                format!("{}: unterminated string", file),
                                Span { line: sl, col: sc },
                            ));
                        }
                        let esc = bytes[i + 1];
                        let ch = match esc {
                            'n' => '\n',
                            't' => '\t',
                            'r' => '\r',
                            'e' => '\u{1b}',
                            '"' => '"',
                            '\\' => '\\',
                            other => {
                                return Err((
                                    format!("{}: unknown escape '\\{}'", file, other),
                                    Span { line: sl, col: sc },
                                ))
                            }
                        };
                        s.push(ch);
                        i += 2;
                        col += 2;
                        continue;
                    }
                    s.push(bytes[i]);
                    i += 1;
                    col += 1;
                }
                if i >= n {
                    return Err((
                        format!("{}: unterminated string", file),
                        Span { line: sl, col: sc },
                    ));
                }
                i += 1;
                col += 1;
                push!(Tok::Str(s), sl, sc);
            }
            c if c.is_ascii_digit() => {
                let start = i;
                while i < n && bytes[i].is_ascii_digit() {
                    i += 1;
                    col += 1;
                }
                // `1.5` is a float; `1..` and `1.x` are not, so a `.` only
                // continues the number when a digit follows it.
                let is_float = i + 1 < n && bytes[i] == '.' && bytes[i + 1].is_ascii_digit();
                if is_float {
                    i += 1;
                    col += 1;
                    while i < n && bytes[i].is_ascii_digit() {
                        i += 1;
                        col += 1;
                    }
                    let s: String = bytes[start..i].iter().collect();
                    push!(Tok::Float(s.parse::<f64>().unwrap_or(0.0)), sl, sc);
                } else {
                    let mut v: i64 = 0;
                    for b in &bytes[start..i] {
                        v = v.wrapping_mul(10).wrapping_add(*b as i64 - '0' as i64);
                    }
                    push!(Tok::Int(v), sl, sc);
                }
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                // `$` is legal inside an identifier but never starts one, so
                // a lifted lambda's name (`owner$0`) is writable — the
                // quarantine set has to be expressible in source — while no
                // author accidentally writes one.
                let mut s = String::new();
                while i < n
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == '_' || bytes[i] == '$')
                {
                    s.push(bytes[i]);
                    i += 1;
                    col += 1;
                }
                let t = match s.as_str() {
                    "fn" => Tok::KwFn,
                    "let" => Tok::KwLet,
                    "return" => Tok::KwReturn,
                    "import" => Tok::KwImport,
                    "from" => Tok::KwFrom,
                    "if" => Tok::KwIf,
                    "else" => Tok::KwElse,
                    "dyn" => Tok::KwDyn,
                    "table" => Tok::KwTable,
                    "extern" => Tok::KwExtern,
                    "memo" => Tok::KwMemo,
                    "len" => Tok::KwLen,
                    "push" => Tok::KwPush,
                    "input" => Tok::KwInput,
                    "while" => Tok::KwWhile,
                    "str" => Tok::KwStr,
                    "int" => Tok::KwInt,
                    "float" => Tok::KwFloat,
                    "in" => Tok::KwIn,
                    "property" => Tok::KwProperty,
                    "for" => Tok::KwFor,
                    "substr" => Tok::KwSubstr,
                    "chars" => Tok::KwChars,
                    "join" => Tok::KwJoin,
                    "read_file" => Tok::KwReadFile,
                    "write_file" => Tok::KwWriteFile,
                    "fail" => Tok::KwFail,
                    "try" => Tok::KwTry,
                    "print" => Tok::KwPrint,
                    _ => Tok::Ident(s),
                };
                push!(t, sl, sc);
            }
            other => {
                return Err((
                    format!("{}: unexpected character '{}'", file, other),
                    Span { line, col },
                ));
            }
        }
    }
    out.push(Token {
        tok: Tok::Eof,
        span: Span { line, col },
    });
    Ok((out, comments))
}
