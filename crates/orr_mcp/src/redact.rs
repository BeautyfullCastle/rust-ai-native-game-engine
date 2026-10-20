//! Targeted credential redaction shared by library failure text and binary diagnostics.

pub(crate) fn text(value: &str, token: Option<&str>) -> String {
    // Project all redaction spans from the original text. Replacing URL text
    // first can hide an exact URL-shaped token; replacing the token first can
    // damage a URL scheme and hide other credentials (for example token="ws").
    let mut spans = Vec::new();
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        spans.extend(value.match_indices(token).map(|(i, _)| (i, i + token.len())));
    }
    url_spans(value, &mut spans);
    spans.sort_unstable();
    let mut out = String::with_capacity(value.len());
    let mut copied = 0;
    for (start, end) in spans {
        if start >= copied {
            out.push_str(&value[copied..start]);
            out.push_str("***");
        }
        copied = copied.max(end);
    }
    out.push_str(&value[copied..]);
    out
}

// Targeted credential hygiene, not general secret detection/DLP. Preserve
// ordinary prose and URL paths, but strip URL userinfo, query and fragment text.
fn url_spans(text: &str, spans: &mut Vec<(usize, usize)>) {
    let delimiter = |c: char| c.is_whitespace() || matches!(c, '\"' | '<' | '>' | '`');
    let mut offset = 0;
    for piece in text.split_inclusive(delimiter) {
        let end = piece.char_indices().next_back().filter(|(_, c)| delimiter(*c)).map_or(piece.len(), |(i, _)| i);
        let chunk = &piece[..end];
        let mut private = None;
        let mut privacy_scanned = false;
        for (marker, _) in chunk.match_indices("://") {
            if private.is_some_and(|start| marker >= start) {
                break;
            }
            let scheme_start = chunk[..marker]
                .char_indices()
                .rev()
                .find(|(_, c)| !c.is_ascii_alphanumeric() && !matches!(c, '+' | '-' | '.'))
                .map_or(0, |(i, c)| i + c.len_utf8());
            if !chunk[scheme_start..marker].starts_with(|c: char| c.is_ascii_alphabetic()) {
                continue;
            }
            let tail_start = marker + 3;
            // Keep the first recognized URL's privacy boundary even when its
            // public path embeds another (possibly invalid) :// sequence.
            if !privacy_scanned {
                private = chunk[tail_start..].find(['?', '#']).map(|i| tail_start + i + 1);
                privacy_scanned = true;
            }
            let tail = &chunk[tail_start..private.map_or(chunk.len(), |i| i - 1)];
            let authority = &tail[..tail.find('/').unwrap_or(tail.len())];
            if let Some(at) = authority.rfind('@') {
                spans.push((offset + tail_start, offset + tail_start + at));
            }
        }
        if let Some(start) = private {
            spans.push((offset + start, offset + chunk.len()));
        }
        offset += piece.len();
    }
}

#[cfg(test)]
mod tests {
    use super::text;

    #[test]
    fn urls_redact_credentials_query_and_fragment_without_breaking_unicode() {
        for input in [
            "오류ws://user:password@host/path?token=query#fragment 끝",
            "😀wss://user:password@host/path?token=query#fragment 끝",
            "https://user:password@host/path?token=query#fragment",
            "ws://user:password@[::1]:7777/path?token=query#fragment",
            "ws://user:password@host/path#fragment",
            "ws://user:p'assword@host/path?token=query#fragment",
            "https://docs.example/path/ws://user:password@host/path",
            "https://docs.example/path/ws://user:password@host/path?token=query#fragment",
            "https://docs.example/path/1://not-a-url?token=query#fragment",
        ] {
            let redacted = text(input, None);
            for secret in ["password", "assword", "query", "fragment"] {
                assert!(!redacted.contains(secret), "{redacted}");
            }
            assert!(redacted.contains("/path"));
        }
        for ordinary in ["ordinary failure", "오류ws://host/path 끝", "https://example.test/a/b", "ws://[::1]:7777/path", "1://not-a-url"] {
            assert_eq!(text(ordinary, None), ordinary);
        }
    }

    #[test]
    fn exact_known_token_is_removed_from_anywhere_in_composed_text() {
        assert_eq!(text("token-秘密 [token-秘密] https://host/token-秘密", Some("token-秘密")), "*** [***] https://host/***");
        assert_eq!(text("ordinary", Some("")), "ordinary");
        assert_eq!(text("echo ws://secret-bearer-value?fixed", Some("ws://secret-bearer-value?fixed")), "echo ***");
        assert_eq!(text("echo ws://user:password@host?private", Some("ws")), "echo ***://***@host?***");
    }
}
