//! Repository-owned local supervision fixture, never a Stormglass game.
use std::os::windows::process::CommandExt;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::Duration,
};
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|v| v == "--probe-descendant") {
        thread::sleep(Duration::from_millis(500));
        fs::write(&args[1], "escaped").unwrap();
        thread::sleep(Duration::from_secs(60));
        return;
    }
    assert_eq!(&args[..4], ["--headless", "--ticks", "60", "--scene"]);
    assert_eq!(args.len(), 5);
    let scene = Path::new(&args[4]);
    let mode = fs::read_to_string(scene).unwrap();
    let root = scene.parent().unwrap();
    match mode.as_str() {
        "shape" => {
            let env: Vec<_> = std::env::vars().collect();
            assert_eq!(env.len(), 2);
            assert!(env
                .iter()
                .all(|(k, _)| k.eq_ignore_ascii_case("SystemRoot")
                    || k.eq_ignore_ascii_case("WINDIR")));
            assert_eq!(std::env::current_dir().unwrap(), root);
            assert_eq!(
                std::io::Read::read(&mut std::io::stdin(), &mut [0u8; 1]).unwrap(),
                0
            );
            println!("shape checked");
        }
        "nonzero" => std::process::exit(23),
        "stderr" => eprintln!("expected probe stderr"),
        "timeout" => thread::sleep(Duration::from_secs(60)),
        "overflow" => {
            std::io::stdout().write_all(&[b'o'; 48]).unwrap();
            std::io::stderr().write_all(&[b'e'; 48]).unwrap();
        }
        "descendant" => {
            let child = Command::new(std::env::current_exe().unwrap())
                .creation_flags(0x0800_0000)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .arg("--probe-descendant")
                .arg(root.join("descendant-escaped"))
                .spawn()
                .unwrap();
            println!("descendant pid={}", child.id());
        }
        "marker" => fs::write(root.join("executed"), "unexpected").unwrap(),
        _ => panic!("unknown test mode"),
    }
}
