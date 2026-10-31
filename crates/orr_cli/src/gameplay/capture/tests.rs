use super::*;
fn fixture() -> (tempfile::TempDir, Options, J, J, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let o = Options {
        path: dir.path().join("frame.png"),
        timeout: Duration::from_secs(2),
        width: 2048,
        height: 2048,
    };
    let id = json!({"game":"fixture","build_id":"0x0000000000000001"});
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[255, 0, 0, 255, 0, 255, 0, 255])
            .unwrap();
    }
    let v = json!({"status":"captured","source":"editor.app_framebuffer","game":id["game"],"build_id":id["build_id"],"mode":"edit","paused":true,"tick":"0","epoch":"1","checksum":"0x0000000000000002","frame_seq":"1","ui_frame":"1","width":2,"height":1,"mime_type":"image/png","png_base64":orr_remote::codec::b64_encode(&bytes),"private":"not emitted"});
    (dir, o, id, v, bytes)
}
struct Mock {
    replies: std::collections::VecDeque<Result<J, CliErr>>,
    calls: Vec<String>,
}
impl Session for Mock {
    fn call(&mut self, m: &str, _: J, _: Duration) -> Result<J, CliErr> {
        self.calls.push(m.into());
        self.replies.pop_front().expect("no retry")
    }
}
#[test]
fn capture_single_request_valid_png_and_allowlist() {
    let (_dir, o, id, v, bytes) = fixture();
    let mut s = Mock {
        replies: [Ok(id), Ok(v)].into(),
        calls: vec![],
    };
    let out = execute(&mut s, &o, Instant::now() + o.timeout).unwrap();
    assert_eq!(s.calls, ["editor.gameplay.describe", "view.screenshot"]);
    assert_eq!(std::fs::read(&o.path).unwrap(), bytes);
    assert!(out.get("png_base64").is_none() && out.get("private").is_none());
}
#[test]
fn capture_rejects_metadata_types_identity_and_limits() {
    let (_dir, o, id, v, _) = fixture();
    for (key, value) in [
        ("game", json!("wrong")),
        ("build_id", json!("wrong")),
        ("mode", json!("wrong")),
        ("paused", json!("true")),
        ("tick", json!(1)),
        ("epoch", json!("01")),
        ("frame_seq", json!(-1)),
        ("ui_frame", J::Null),
        ("checksum", json!("bad")),
        ("width", json!(2049)),
        ("height", json!(0)),
        ("width", json!(2.5)),
        ("mime_type", json!("text/plain")),
        ("source", json!("fake")),
        ("status", json!("queued")),
    ] {
        let mut bad = v.clone();
        bad[key] = value;
        assert!(validate(&bad, &id, &o).is_err(), "{key}");
    }
    let mut bad = v.clone();
    bad["width"] = json!(2048);
    bad["height"] = json!(2048);
    assert!(validate(&bad, &id, &o).is_err());
    let small = Options { width: 1, ..o };
    assert!(validate(&v, &id, &small).is_err());
}
#[test]
fn capture_rejects_base64_and_png_bounds_before_save() {
    let (_dir, o, id, v, bytes) = fixture();
    for data in [
        "!".into(),
        "A".repeat(BASE64_LIMIT + 1),
        orr_remote::codec::b64_encode(&vec![0; PNG_LIMIT + 1]),
        orr_remote::codec::b64_encode(b"PNG"),
    ] {
        let mut bad = v.clone();
        bad["png_base64"] = json!(data);
        assert!(validate(&bad, &id, &o).is_err());
    }
    let mut truncated = bytes.clone();
    truncated.truncate(bytes.len() - 1);
    let mut corrupt = bytes.clone();
    corrupt[29] ^= 1;
    let mut trailing = bytes.clone();
    trailing.push(0);
    let mut duplicate_end = bytes.clone();
    duplicate_end.extend_from_slice(&bytes[bytes.len() - 12..]);
    let mut garbage_end = bytes.clone();
    garbage_end.extend_from_slice(b"garbage");
    garbage_end.extend_from_slice(&bytes[bytes.len() - 12..]);
    let concatenated = [bytes.as_slice(), bytes.as_slice()].concat();
    for data in [
        truncated,
        corrupt,
        trailing,
        duplicate_end,
        garbage_end,
        concatenated,
    ] {
        let mut bad = v.clone();
        bad["png_base64"] = json!(orr_remote::codec::b64_encode(&data));
        assert!(validate(&bad, &id, &o).is_err());
    }
    let mut mismatch = v;
    mismatch["width"] = json!(1);
    assert!(validate(&mismatch, &id, &o).is_err());
    assert!(!o.path.exists());
}
#[test]
fn capture_failure_does_not_retry_or_save() {
    let (_dir, o, id, _, _) = fixture();
    let mut s = Mock {
        replies: [Ok(id), Err(failed("busy"))].into(),
        calls: vec![],
    };
    assert!(execute(&mut s, &o, Instant::now() + o.timeout).is_err());
    assert_eq!(s.calls.len(), 2);
    assert!(!o.path.exists());
    let mut empty = Mock {
        replies: [].into(),
        calls: vec![],
    };
    assert!(execute(&mut empty, &o, Instant::now()).is_err());
    assert!(empty.calls.is_empty());
}
#[test]
fn capture_path_preflight_and_atomic_collision_cleanup() {
    let (dir, o, _, _, bytes) = fixture();
    assert!(validate_path(&o.path).is_ok());
    std::fs::write(&o.path, b"original").unwrap();
    assert!(validate_path(&o.path).is_err());
    assert!(save(&o.path, &bytes).is_err());
    assert_eq!(std::fs::read(&o.path).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(validate_path(dir.path()).is_err());
    assert!(validate_path(&dir.path().join("missing/out.png")).is_err());
}
#[cfg(unix)]
#[test]
fn capture_rejects_symlink_destination_and_parent() {
    use std::os::unix::fs::symlink;
    let (dir, o, _, _, _) = fixture();
    symlink(dir.path().join("missing"), &o.path).unwrap();
    assert!(validate_path(&o.path).is_err());
    let alias = dir.path().join("alias");
    symlink(dir.path(), &alias).unwrap();
    assert!(validate_path(&alias.join("new.png")).is_err());
}
#[test]
fn capture_options_leading_flags_and_no_force() {
    let (_dir, o, _, _, _) = fixture();
    let path = o.path.to_str().unwrap();
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let valid = args(&["--out", path, "--timeout-ms", "1000", "capture"]);
    assert_eq!(command(&valid).unwrap(), "capture");
    assert!(options(&valid).is_ok());
    for a in [
        vec!["capture"],
        vec!["capture", "--out", path, "--force"],
        vec!["capture", "--out", path, "--max-width", "0"],
        vec!["capture", "--out", path, "--timeout-ms", "99"],
        vec!["capture", "--out", path, "--out", path],
        vec!["capture", "extra", "--out", path],
    ] {
        assert!(options(&args(&a)).is_err());
    }
}

#[test]
fn capture_rejects_ancillary_crc_and_zlib_adler() {
    fn crc(data: &[u8]) -> u32 {
        let mut value = !0u32;
        for byte in data {
            value ^= u32::from(*byte);
            for _ in 0..8 {
                value = (value >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(value & 1));
            }
        }
        !value
    }
    let (_dir, o, id, v, bytes) = fixture();
    let mut ancillary = bytes[..bytes.len() - 12].to_vec();
    ancillary.extend_from_slice(&3u32.to_be_bytes());
    ancillary.extend_from_slice(b"tEXtk\0v");
    ancillary.extend_from_slice(&(crc(b"tEXtk\0v") ^ 1).to_be_bytes());
    ancillary.extend_from_slice(&bytes[bytes.len() - 12..]);
    let mut adler = bytes.clone();
    let mut at = 8;
    let mut changed = false;
    while at + 12 <= adler.len() {
        let len = u32::from_be_bytes(adler[at..at + 4].try_into().unwrap()) as usize;
        if &adler[at + 4..at + 8] == b"IDAT" {
            assert!(len >= 4);
            adler[at + 8 + len - 1] ^= 1;
            let sum = crc(&adler[at + 4..at + 8 + len]);
            adler[at + 8 + len..at + 12 + len].copy_from_slice(&sum.to_be_bytes());
            changed = true;
            break;
        }
        at += 12 + len;
    }
    assert!(changed);
    for data in [ancillary, adler] {
        let mut bad = v.clone();
        bad["png_base64"] = json!(orr_remote::codec::b64_encode(&data));
        assert!(validate(&bad, &id, &o).is_err());
    }
}

#[test]
fn capture_redacts_decoded_strings_before_json_escaping() {
    let token = "test-only-quote\"-slash\\-newline\n";
    let url = "ws://127.0.0.1:1";
    let ctx = Ctx {
        bridge: orr_mcp::Bridge::to_url(url, Some(token.into()), Duration::from_secs(1)),
        json: true,
        url: url.into(),
        token: Some(token.into()),
    };
    let result = json!({"game": token, "local_png_path": format!("prefix/{token}/capture.png"), "source":"token=query-secret&safe=yes", "width":2});
    let encoded = output_json(&ctx, result);
    let parsed: J = serde_json::from_str(&encoded).unwrap();
    assert_eq!(parsed["game"], "***");
    assert_eq!(parsed["local_png_path"], "prefix/***/capture.png");
    assert_eq!(parsed["source"], "token=***&safe=yes");
    assert_eq!(parsed["width"], 2);
    assert!(!encoded.contains("test-only-quote") && !encoded.contains("query-secret"));
}
