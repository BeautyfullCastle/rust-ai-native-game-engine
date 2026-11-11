#![cfg(feature = "stormglass-embedded-distribution")]
//! Actual CLI argument/error boundary. These cases must not read or execute inputs.
use std::process::Command;
fn invoke(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_orr_publish_stormglass"))
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn help_and_usage_errors_are_complete_and_machine_readable() {
    let help = invoke(&["--help"]);
    assert!(help.status.success());
    assert!(help.stderr.is_empty());
    let text = String::from_utf8(help.stdout).unwrap();
    for word in [
        "--trusted-runtime",
        "--dry-run",
        "no overwrite",
        "no execution",
    ] {
        // Help explicitly describes dry-run and no-overwrite semantics in natural language.
        let alternative = match word {
            "no overwrite" => "never overwrites",
            "no execution" => "never executes",
            other => other,
        };
        assert!(text.contains(alternative), "{text}");
    }
    for args in [
        vec![],
        vec!["--force"],
        vec!["--runtime"],
        vec!["--trusted-runtime", "--trusted-runtime"],
    ] {
        let output = invoke(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["kind"], "usage");
        assert!(error["error"].as_str().unwrap().len() > 5);
    }
}
#[test]
fn missing_trust_and_invalid_paths_fail_before_operation() {
    let pin = "a".repeat(64);
    let base = [
        "--runtime",
        r"C:\never-open\runtime.exe",
        "--scene",
        r"C:\never-open\scene.yaml",
        "--runtime-sha256",
        &pin,
        "--output",
        r"C:\never-open\result.exe",
    ];
    let output = invoke(&base);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(error["error"].as_str().unwrap().contains("trusted-runtime"));
    let mut args = base.to_vec();
    args.extend([
        "--trusted-runtime",
        "--output",
        r"C:\never-open\replacement.exe",
    ]);
    let output = invoke(&args);
    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(error["error"].as_str().unwrap().contains("duplicate"));
}
