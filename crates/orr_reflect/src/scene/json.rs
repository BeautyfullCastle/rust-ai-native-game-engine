//! A tiny JSON tree with a fixed pretty printer, for the schema output.
//!
//! Object keys keep insertion order, so output is the same on every run.
//! Numbers are pre-formatted text (fixed-point bounds are exact decimals).

/// A JSON value.
#[derive(Clone, Debug)]
pub(super) enum Json {
    Bool(bool),
    /// Number as text.
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub(super) fn str(s: &str) -> Json {
        Json::Str(s.to_string())
    }
    pub(super) fn num(n: impl ToString) -> Json {
        Json::Num(n.to_string())
    }
    pub(super) fn obj(fields: Vec<(&str, Json)>) -> Json {
        Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    /// Adds or replaces a key on an object.
    pub(super) fn with(mut self, key: &str, value: Json) -> Json {
        if let Json::Obj(fields) = &mut self {
            match fields.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value,
                None => fields.push((key.to_string(), value)),
            }
        }
        self
    }

    /// Two-space pretty print with a final newline.
    pub(super) fn pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    fn is_scalar(&self) -> bool {
        matches!(self, Json::Bool(_) | Json::Num(_) | Json::Str(_))
    }

    fn write(&self, out: &mut String, indent: usize) {
        match self {
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => out.push_str(n),
            Json::Str(s) => quote(out, s),
            Json::Arr(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                } else if items.iter().all(Json::is_scalar) && items.len() <= 8 {
                    out.push('[');
                    for (i, it) in items.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        it.write(out, indent);
                    }
                    out.push(']');
                } else {
                    out.push_str("[\n");
                    for (i, it) in items.iter().enumerate() {
                        pad(out, indent + 2);
                        it.write(out, indent + 2);
                        if i + 1 < items.len() {
                            out.push(',');
                        }
                        out.push('\n');
                    }
                    pad(out, indent);
                    out.push(']');
                }
            }
            Json::Obj(fields) => {
                if fields.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                for (i, (k, v)) in fields.iter().enumerate() {
                    pad(out, indent + 2);
                    quote(out, k);
                    out.push_str(": ");
                    v.write(out, indent + 2);
                    if i + 1 < fields.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                pad(out, indent);
                out.push('}');
            }
        }
    }
}

fn pad(out: &mut String, n: usize) {
    for _ in 0..n {
        out.push(' ');
    }
}

fn quote(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}
