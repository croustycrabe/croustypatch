//! Tiny boolean expression language for deterministic ("no AI") rules and policy scopes.
//!
//! ```text
//! cvss_score >= 8.0 AND asset_environment == 'production' AND port_exposed_to_internet == true
//! (criticality == 'high' OR 'pci' in tags) AND NOT has_staging
//! ```
//! Operators: `== != < <= > >=`, `in`, `AND`/`OR`/`NOT` (also `&&`, `||`, `!`), parentheses.
//! Unknown identifiers evaluate to null; any comparison with null is false (never an error), so a
//! rule referring to an attribute a host lacks simply does not match.
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<String>),
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Num(n)
    }
}
impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Num(n as f64)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}
impl From<Vec<String>> for Value {
    fn from(l: Vec<String>) -> Self {
        Value::List(l)
    }
}

pub type Context = HashMap<String, Value>;

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
#[error("expression error at {pos}: {msg}")]
pub struct ExprError {
    pub pos: usize,
    pub msg: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Num(f64),
    Str(String),
    Op(&'static str), // == != < <= > >= 
    And,
    Or,
    Not,
    In,
    LParen,
    RParen,
    True,
    False,
}

fn lex(src: &str) -> Result<Vec<(usize, Tok)>, ExprError> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = vec![];
    let err = |pos, msg: &str| ExprError { pos, msg: msg.into() };
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        match c {
            c if c.is_whitespace() => i += 1,
            '(' => {
                out.push((start, Tok::LParen));
                i += 1
            }
            ')' => {
                out.push((start, Tok::RParen));
                i += 1
            }
            '\'' | '"' => {
                i += 1;
                let mut s = String::new();
                loop {
                    match chars.get(i) {
                        None => return Err(err(start, "unterminated string")),
                        Some(&q) if q == c => {
                            i += 1;
                            break;
                        }
                        Some(&ch) => {
                            s.push(ch);
                            i += 1
                        }
                    }
                }
                out.push((start, Tok::Str(s)));
            }
            '=' | '!' | '<' | '>' => {
                let two = chars.get(i + 1) == Some(&'=');
                let tok = match (c, two) {
                    ('=', true) => Tok::Op("=="),
                    ('!', true) => Tok::Op("!="),
                    ('<', true) => Tok::Op("<="),
                    ('>', true) => Tok::Op(">="),
                    ('<', false) => Tok::Op("<"),
                    ('>', false) => Tok::Op(">"),
                    ('!', false) => Tok::Not,
                    _ => return Err(err(start, "use '==' for equality")),
                };
                i += if two { 2 } else { 1 };
                out.push((start, tok));
            }
            '&' | '|' => {
                if chars.get(i + 1) != Some(&c) {
                    return Err(err(start, "expected '&&' or '||'"));
                }
                out.push((start, if c == '&' { Tok::And } else { Tok::Or }));
                i += 2;
            }
            c if c.is_ascii_digit() || (c == '-' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) => {
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                out.push((start, Tok::Num(text.parse().map_err(|_| err(start, "bad number"))?)));
            }
            c if c.is_alphabetic() || c == '_' => {
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                    i += 1;
                }
                let w: String = chars[start..i].iter().collect();
                let tok = match w.to_ascii_lowercase().as_str() {
                    "and" => Tok::And,
                    "or" => Tok::Or,
                    "not" => Tok::Not,
                    "in" => Tok::In,
                    "true" => Tok::True,
                    "false" => Tok::False,
                    _ => Tok::Ident(w),
                };
                out.push((start, tok));
            }
            _ => return Err(err(start, &format!("unexpected character '{c}'"))),
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Lit(Value),
    Var(String),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Cmp(&'static str, Box<Expr>, Box<Expr>),
    In(Box<Expr>, Box<Expr>),
}

struct Parser {
    toks: Vec<(usize, Tok)>,
    i: usize,
    end: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|(_, t)| t)
    }
    fn pos(&self) -> usize {
        self.toks.get(self.i).map(|(p, _)| *p).unwrap_or(self.end)
    }
    fn err<T>(&self, msg: &str) -> Result<T, ExprError> {
        Err(ExprError { pos: self.pos(), msg: msg.into() })
    }
    fn or(&mut self) -> Result<Expr, ExprError> {
        let mut l = self.and()?;
        while self.peek() == Some(&Tok::Or) {
            self.i += 1;
            l = Expr::Or(Box::new(l), Box::new(self.and()?));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<Expr, ExprError> {
        let mut l = self.not()?;
        while self.peek() == Some(&Tok::And) {
            self.i += 1;
            l = Expr::And(Box::new(l), Box::new(self.not()?));
        }
        Ok(l)
    }
    fn not(&mut self) -> Result<Expr, ExprError> {
        if self.peek() == Some(&Tok::Not) {
            self.i += 1;
            return Ok(Expr::Not(Box::new(self.not()?)));
        }
        self.cmp()
    }
    fn cmp(&mut self) -> Result<Expr, ExprError> {
        let l = self.atom()?;
        match self.peek().cloned() {
            Some(Tok::Op(op)) => {
                self.i += 1;
                Ok(Expr::Cmp(op, Box::new(l), Box::new(self.atom()?)))
            }
            Some(Tok::In) => {
                self.i += 1;
                Ok(Expr::In(Box::new(l), Box::new(self.atom()?)))
            }
            _ => Ok(l),
        }
    }
    fn atom(&mut self) -> Result<Expr, ExprError> {
        let t = self.peek().cloned();
        match t {
            Some(Tok::LParen) => {
                self.i += 1;
                let e = self.or()?;
                if self.peek() != Some(&Tok::RParen) {
                    return self.err("expected ')'");
                }
                self.i += 1;
                Ok(e)
            }
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(Expr::Lit(Value::Num(n)))
            }
            Some(Tok::Str(s)) => {
                self.i += 1;
                Ok(Expr::Lit(Value::Str(s)))
            }
            Some(Tok::True) => {
                self.i += 1;
                Ok(Expr::Lit(Value::Bool(true)))
            }
            Some(Tok::False) => {
                self.i += 1;
                Ok(Expr::Lit(Value::Bool(false)))
            }
            Some(Tok::Ident(n)) => {
                self.i += 1;
                Ok(Expr::Var(n))
            }
            _ => self.err("expected a value"),
        }
    }
}

pub fn parse(src: &str) -> Result<Expr, ExprError> {
    let toks = lex(src)?;
    if toks.is_empty() {
        return Err(ExprError { pos: 0, msg: "empty expression".into() });
    }
    let mut p = Parser { toks, i: 0, end: src.chars().count() };
    let e = p.or()?;
    if p.i != p.toks.len() {
        return p.err("unexpected trailing input");
    }
    Ok(e)
}

fn value(e: &Expr, ctx: &Context) -> Value {
    match e {
        Expr::Lit(v) => v.clone(),
        Expr::Var(n) => ctx.get(n).cloned().unwrap_or(Value::Null),
        other => Value::Bool(truthy(other, ctx)),
    }
}

fn truthy(e: &Expr, ctx: &Context) -> bool {
    match e {
        Expr::Not(a) => !truthy(a, ctx),
        Expr::And(a, b) => truthy(a, ctx) && truthy(b, ctx),
        Expr::Or(a, b) => truthy(a, ctx) || truthy(b, ctx),
        Expr::Cmp(op, a, b) => compare(op, &value(a, ctx), &value(b, ctx)),
        Expr::In(a, b) => match (value(a, ctx), value(b, ctx)) {
            (Value::Str(s), Value::List(l)) => l.iter().any(|x| x.eq_ignore_ascii_case(&s)),
            (Value::Str(s), Value::Str(h)) => h.contains(&s),
            _ => false,
        },
        other => matches!(value(other, ctx), Value::Bool(true)),
    }
}

fn compare(op: &str, a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (Null, _) | (_, Null) => false,
        (Num(x), Num(y)) => match op {
            "==" => (x - y).abs() < f64::EPSILON,
            "!=" => (x - y).abs() >= f64::EPSILON,
            "<" => x < y,
            "<=" => x <= y,
            ">" => x > y,
            ">=" => x >= y,
            _ => false,
        },
        (Str(x), Str(y)) => match op {
            "==" => x.eq_ignore_ascii_case(y),
            "!=" => !x.eq_ignore_ascii_case(y),
            _ => false,
        },
        (Bool(x), Bool(y)) => match op {
            "==" => x == y,
            "!=" => x != y,
            _ => false,
        },
        _ => false,
    }
}

pub fn eval(e: &Expr, ctx: &Context) -> bool {
    truthy(e, ctx)
}

/// Parse and evaluate in one go.
pub fn eval_str(src: &str, ctx: &Context) -> Result<bool, ExprError> {
    Ok(eval(&parse(src)?, ctx))
}
