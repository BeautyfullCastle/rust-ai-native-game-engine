use super::*;
fn args() -> Vec<OsString> {
    let mut a = Vec::new();
    for cue in CUES {
        a.push("--cue".into());
        a.push(format!("{cue}=sounds/{cue}.pcm").into());
    }
    a.extend(["--license", "LICENSE", "--output", "bank"].map(OsString::from));
    a
}
#[test]
fn fixed_cues_and_closed_bounded_options() {
    let o = Options::parse(args()).unwrap();
    assert_eq!(o.cues.len(), 10);
    for key in ["--importer", "--command", "--license"] {
        let mut a = args();
        a[0] = key.into();
        assert!(Options::parse(a).is_err());
    }
    let mut a = args();
    a[3] = a[1].clone();
    assert!(Options::parse(a).is_err());
    let mut a = args();
    a[1] = "attack-light=".into();
    assert!(Options::parse(a).is_err());
    let mut a = args();
    a[1] = "x".repeat(65536).into();
    assert!(Options::parse(a).is_err());
    assert!(Options::parse(std::iter::repeat(OsString::from("--cue"))).is_err());
}
#[test]
fn cue_path_keeps_first_equals_and_os_bytes() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let mut a = args();
    let raw = b"attack-light=.orr/x=\xff.pcm";
    a[1] = OsString::from_vec(raw.to_vec());
    let o = Options::parse(a).unwrap();
    assert_eq!(o.cues[0].1.as_os_str().as_bytes(), b".orr/x=\xff.pcm");
    assert!(absolute(&o.cues[0].1).is_ok());
    assert!(absolute(Path::new("../escape")).is_err());
    assert!(absolute(Path::new(&"x".repeat(4097))).is_err());
}
#[test]
fn invalid_input_never_creates_output() {
    let t = tempfile::tempdir().unwrap();
    let license = t.path().join("LICENSE");
    std::fs::write(&license, b"provided").unwrap();
    let mut cues = Vec::new();
    for cue in CUES {
        let p = t.path().join(format!("{cue}.pcm"));
        std::fs::write(&p, [0u8; 8]).unwrap();
        cues.push((cue.into(), p));
    }
    let o = Options {
        cues,
        license,
        output: t.path().join("out"),
    };
    assert!(create(&o, InputKind::Pcm).is_err());
    assert!(!o.output.exists());
}
