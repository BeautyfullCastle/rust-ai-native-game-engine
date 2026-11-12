//! Template inspection only, not HTML sanitization or a browser DOM parser.
//! Unlike tolerant Python HTMLParser, malformed tokens, NUL, empty/valueless
//! IDs and unterminated script/style are refused. Only script/style use raw text;
//! other contexts remain conservatively scanned, matching the legacy ID fence.
use html5gum::{
    emitters::callback::{CallbackEmitter, CallbackEvent},
    Span, State, Tokenizer,
};
use std::collections::BTreeSet;

const MAX_TEMPLATE: usize = 64 * 1024 * 1024;
const MAX_HTML: usize = 256 * 1024 * 1024;
const MAX_TAGS: usize = 100_000;
const STYLE: &str = "<link rel=\"stylesheet\" href=\"style.css\">";
const SCRIPT: &str = "<script type=\"module\" src=\"game.js\"></script>";
const RESERVED: &[&str] = &[
    "authored-audio-bank",
    "wasm-core",
    "tower-atlas",
    "enemy-atlas",
    "character-atlas-meta",
    "game-module",
    "authored-scene",
    "authored-scene-receipt",
    "combat-animation-manifest",
];

pub(super) struct CheckedTemplate {
    original: String,
}
impl CheckedTemplate {
    pub(super) fn render(&self, css: &str, embedded: &str, js: &str) -> Result<String, String> {
        // The embedded fragment is supplied only by the typed offline hub, not
        // arbitrary author input. CSS/JS terminators are independently refused.
        if contains_ascii_case(css.as_bytes(), b"</style")
            || contains_ascii_case(js.as_bytes(), b"</script")
        {
            return Err("CSS or JS would terminate its element".into());
        }
        let style_size = css
            .len()
            .checked_add("<style></style>".len())
            .ok_or("HTML size overflow")?;
        let script_size = embedded
            .len()
            .checked_add(js.len())
            .and_then(|n| {
                n.checked_add("<script type=\"module\" id=\"game-module\"></script>".len())
            })
            .ok_or("HTML size overflow")?;
        let size = self
            .original
            .len()
            .checked_sub(STYLE.len() + SCRIPT.len())
            .and_then(|n| n.checked_add(style_size))
            .and_then(|n| n.checked_add(script_size))
            .ok_or("HTML size overflow")?;
        if size > MAX_HTML {
            return Err("HTML exceeds 256 MiB".into());
        }
        // Do not search the generated replacement text for the other anchor.
        let a = self.original.find(STYLE).ok_or("lost stylesheet anchor")?;
        let b = self.original.find(SCRIPT).ok_or("lost module anchor")?;
        let mut pieces = [(a, STYLE.len(), true), (b, SCRIPT.len(), false)];
        pieces.sort_by_key(|p| p.0);
        let mut out = String::with_capacity(size);
        let mut offset = 0;
        for (start, len, style) in pieces {
            out.push_str(&self.original[offset..start]);
            if style {
                out.push_str("<style>");
                out.push_str(css);
                out.push_str("</style>");
            } else {
                out.push_str(embedded);
                out.push_str("<script type=\"module\" id=\"game-module\">");
                out.push_str(js);
                out.push_str("</script>");
            }
            offset = start + len;
        }
        out.push_str(&self.original[offset..]);
        Ok(out)
    }
}
fn contains_ascii_case(bytes: &[u8], needle: &[u8]) -> bool {
    bytes
        .windows(needle.len())
        .any(|v| v.eq_ignore_ascii_case(needle))
}

struct Inspection<'a> {
    input: &'a str,
    ids: BTreeSet<Vec<u8>>,
    tag: Vec<u8>,
    start: usize,
    open: bool,
    id_seen: bool,
    current_id: bool,
    id: Vec<u8>,
    raw: Option<Vec<u8>>,
    tags: usize,
    style: bool,
    script: bool,
}
impl Inspection<'_> {
    fn event(
        &mut self,
        event: CallbackEvent<'_>,
        span: Span<usize>,
    ) -> Result<Option<State>, String> {
        match event {
            CallbackEvent::OpenStartTag { name } => {
                if self.open {
                    return Err("overlapping start tags".into());
                }
                self.tags += 1;
                if self.tags > MAX_TAGS {
                    return Err("template tag budget exceeded".into());
                }
                self.open = true;
                self.tag.clear();
                self.tag.extend_from_slice(name);
                self.start = span.start;
                self.id_seen = false;
                self.current_id = false;
                self.id.clear();
            }
            CallbackEvent::AttributeName { name } => {
                if !self.open {
                    return Err("attributes outside a start tag".into());
                }
                self.current_id = name == b"id";
                if self.current_id {
                    if self.id_seen {
                        return Err("duplicate id attribute".into());
                    }
                    self.id_seen = true;
                }
            }
            CallbackEvent::AttributeValue { value } => {
                if self.current_id {
                    self.id.extend_from_slice(value);
                }
            }
            CallbackEvent::CloseStartTag { self_closing } => {
                if !self.open {
                    return Err("unexpected start tag close".into());
                }
                self.open = false;
                self.current_id = false;
                if self.id_seen {
                    if self.id.is_empty() {
                        return Err("empty or valueless id is unsupported".into());
                    }
                    let id = std::str::from_utf8(&self.id).map_err(|_| "invalid decoded id")?;
                    if RESERVED.contains(&id) || id.starts_with("authored-animation-") {
                        return Err("reserved embedded ID".into());
                    }
                    if !self.ids.insert(self.id.clone()) {
                        return Err("duplicate HTML ID".into());
                    }
                }
                let tag = self
                    .input
                    .get(self.start..span.end)
                    .ok_or("invalid template token span")?;
                if tag == STYLE {
                    self.style = true;
                }
                if self.tag == b"script"
                    && self
                        .input
                        .get(self.start..)
                        .is_some_and(|s| s.starts_with(SCRIPT))
                {
                    self.script = true;
                }
                if matches!(self.tag.as_slice(), b"script" | b"style") {
                    // A self-closing raw-text tag is interpreted inconsistently
                    // by HTMLParser and browsers. Refuse rather than under-scan.
                    if self_closing {
                        return Err("self-closing script/style unsupported".into());
                    }
                    self.raw = Some(self.tag.clone());
                    // Python's script and style CDATA both end at the first
                    // matching end tag, rather than HTML5 script escape states.
                    return Ok(Some(State::RawText));
                }
            }
            CallbackEvent::EndTag { name } => {
                if self.raw.as_deref() == Some(name) {
                    self.raw = None;
                }
            }
            CallbackEvent::Error(e) => return Err(format!("malformed template token: {e:?}")),
            _ => {}
        }
        Ok(None)
    }
}
pub(super) fn inspect(bytes: &[u8]) -> Result<CheckedTemplate, String> {
    if bytes.len() > MAX_TEMPLATE || bytes.contains(&0) {
        return Err("template exceeds bound or contains NUL".into());
    }
    let input = std::str::from_utf8(bytes).map_err(|_| "template must be UTF-8")?;
    if input.matches(STYLE).count() != 1 || input.matches(SCRIPT).count() != 1 {
        return Err("expected exactly one literal stylesheet and module anchor".into());
    }
    let mut inspection = Inspection {
        input,
        ids: BTreeSet::new(),
        tag: Vec::new(),
        start: 0,
        open: false,
        id_seen: false,
        current_id: false,
        id: Vec::new(),
        raw: None,
        tags: 0,
        style: false,
        script: false,
    };
    {
        let emitter = CallbackEmitter::new(|event: CallbackEvent<'_>, span: Span<usize>| {
            match inspection.event(event, span) {
                Ok(None) => None,
                other => Some(other),
            }
        });
        // Automatic HTML5 title/textarea/noscript state switching stays off.
        let mut tokenizer = Tokenizer::new_with_emitter(input, emitter);
        while let Some(token) = tokenizer.next() {
            let event = token.map_err(|e| format!("template read failure: {e:?}"))??;
            if let Some(state) = event {
                tokenizer.set_state(state);
            }
        }
    }
    if inspection.open || inspection.raw.is_some() {
        return Err("unterminated template tag or raw text".into());
    }
    if !inspection.style || !inspection.script {
        return Err("anchors must be actual tags outside comments/raw text".into());
    }
    Ok(CheckedTemplate {
        original: input.into(),
    })
}
#[cfg(test)]
#[path = "html_template_tests.rs"]
mod tests;
