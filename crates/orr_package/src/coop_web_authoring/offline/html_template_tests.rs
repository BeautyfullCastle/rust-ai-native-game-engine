use super::*;
fn template(extra: &str) -> String {
    format!("<!doctype html><html><head>{STYLE}</head><body>{extra}{SCRIPT}</body></html>")
}
fn admits(extra: &str) -> bool {
    inspect(template(extra).as_bytes()).is_ok()
}
#[test]
fn real_template_and_render_preserve_source_outside_exact_anchors() {
    let source = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../orr_coop_defense_web/web/index.html"
    ));
    let checked = inspect(source).unwrap();
    let out = checked
        .render(
            "body { color: red; }",
            "<script id=\"verified-by-hub\"></script>",
            "const ok = 1;",
        )
        .unwrap();
    let expected=std::str::from_utf8(source).unwrap().replace(STYLE,"<style>body { color: red; }</style>").replace(SCRIPT,"<script id=\"verified-by-hub\"></script><script type=\"module\" id=\"game-module\">const ok = 1;</script>");
    assert_eq!(out, expected);
    assert!(checked.render("x</STYLE>", "", "").is_err());
    assert!(checked.render("", "", "x</ScRiPt>").is_err());
}
#[test]
fn ids_are_case_sensitive_but_attribute_names_fold_and_entities_decode() {
    assert!(admits(
        "<DIV ID=Alpha></DIV><div id=alpha></div><img id='picture'/><p class=x CLASS=y>"
    ));
    for invalid in [
        "<div id=a ID=b>",
        "<div ID=a id=a>",
        "<div id=a></div><p id=a>",
        "<div id='a&#98;'></div><p id=ab>",
        "<div id='wasm&#45;core'>",
        "<div id=authored-animation-other>",
        "<div id=combat-animation-manifest>",
        "<div id='x&amp;y'></div><p id='x&#38;y'>",
        "<img id=a/><p id=a/>",
    ] {
        assert!(!admits(invalid), "{invalid}");
    }
    assert!(admits("<div id=WASM-CORE>"));
}
#[test]
fn comments_and_only_script_style_raw_text_are_not_scanned_as_tags() {
    assert!(admits("<!-- <div id=wasm-core> --><script>const s = '<div id=wasm-core>'; &bogus;</script><STYLE>p::after {content:'<p id=wasm-core>'}</STYLE>"));
    // Deliberately conservative outside the two Python raw-text elements.
    for tag in ["textarea", "title", "noscript", "xmp"] {
        assert!(
            !admits(&format!("<{tag}><i id=wasm-core></{tag}>")),
            "{tag}"
        );
    }
    assert!(!admits("<script/>"));
    assert!(!admits("<style/>"));
}
#[test]
fn anchors_must_be_real_tags_not_comments_or_raw_text() {
    for source in [
        format!("{STYLE}<!-- {SCRIPT} -->"),
        format!("<!-- {STYLE} -->{SCRIPT}"),
        format!("{STYLE}<style>{SCRIPT}</style>"),
        format!("<script>{STYLE}</script>{SCRIPT}"),
        format!("{STYLE}{SCRIPT}{SCRIPT}"),
    ] {
        assert!(inspect(source.as_bytes()).is_err(), "{source}");
    }
    // Attribute text containing an anchor is not a tag occurrence either.
    assert!(inspect(format!("<p title='{STYLE}'></p>{SCRIPT}").as_bytes()).is_err());
}
#[test]
fn malformed_empty_nul_and_missing_values_fail_closed() {
    for invalid in [
        "<p id>",
        "<p id=''>",
        "<p id=x\0>",
        "<p id='unterminated",
        "<p id=x",
        "<script>never closed",
        "<style>never closed",
        "<!-- never closed",
        "</p id=x>",
    ] {
        assert!(!admits(invalid), "{invalid:?}");
    }
    assert!(inspect(b"\xff").is_err());
    let mut large = vec![b' '; MAX_TEMPLATE + 1];
    large[..STYLE.len()].copy_from_slice(STYLE.as_bytes());
    assert!(inspect(&large).is_err());
}
#[test]
fn rendering_never_replaces_an_anchor_inside_generated_content() {
    let input = format!("{STYLE}--{SCRIPT}");
    let checked = inspect(input.as_bytes()).unwrap();
    let css = format!("/* {SCRIPT} */");
    let out = checked.render(&css, "", "ok()").unwrap();
    assert_eq!(
        out,
        format!("<style>{css}</style>--<script type=\"module\" id=\"game-module\">ok()</script>")
    );
    assert!(admits("<p id='safe' data-note='é'>"));
}
