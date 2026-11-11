use super::*;
use std::{cell::Cell, io::Cursor};
const EMPTY: &str = "schema: orr.scene/1\nentities: {}\n";
fn args() -> [OsString; 1] {
    ["--stdin".into()]
}

#[test]
fn defaults_and_repeatability_use_production_initial_state() {
    let a = report(&args(), EMPTY.as_bytes()).unwrap();
    assert_eq!(a, report(&args(), EMPTY.as_bytes()).unwrap());
    assert!(a.contains("\"starting_gold\":150,\"reactor_hp\":30,\"last_wave\":10"));
    assert!(a.contains("\"pair_counts\":[10, 12, 14, 16, 18, 20, 22, 24, 26, 28]"));
    let c = orr_games::coop_defense_game::DefenseConfig::default();
    let sim = Simulation::<DefenseGame>::with_build_id(
        c,
        TICK_RATE,
        u64::from(c.seed),
        SCENARIO_BUILD_ID,
    );
    assert!(a.contains(&format!(
        "\"initial_checksum\":\"0x{:016x}\"",
        sim.checksum()
    )));
}

#[test]
fn usage_and_help_do_not_read_stdin() {
    struct NoRead;
    impl Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("must not read input")
        }
    }
    for values in [
        vec![],
        vec!["--file"],
        vec!["--stdin", "extra"],
        vec!["--help", "--stdin"],
    ] {
        let args = values.into_iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(report(&args, NoRead), Err(Failure::Usage));
    }
    assert_eq!(report(&["--help".into()], NoRead).unwrap(), HELP);
}

#[test]
fn input_budget_is_exact_and_reads_only_one_extra_byte() {
    let valid = format!("{EMPTY}#{}", "x".repeat(MAX_SOURCE_BYTES - EMPTY.len() - 1));
    assert_eq!(valid.len(), MAX_SOURCE_BYTES);
    assert!(report(&args(), valid.as_bytes()).is_ok());
    let read = Cell::new(0);
    struct Count<'a> {
        read: &'a Cell<usize>,
        bytes: Cursor<Vec<u8>>,
    }
    impl Read for Count<'_> {
        fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
            let n = self.bytes.read(target)?;
            self.read.set(self.read.get() + n);
            Ok(n)
        }
    }
    let source = Count {
        read: &read,
        bytes: Cursor::new(vec![b'x'; MAX_SOURCE_BYTES * 2]),
    };
    assert_eq!(report(&args(), source), Err(Failure::TooLarge));
    assert_eq!(read.get(), MAX_SOURCE_BYTES + 1);
}

#[test]
fn invalid_input_uses_stable_private_error_classes() {
    assert_eq!(report(&args(), &[0xff][..]), Err(Failure::InvalidUtf8));
    for bad in [
        "",
        "private-token",
        "schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Campaign: {last_wave: 11}}\n",
    ] {
        assert_eq!(report(&args(), bad.as_bytes()), Err(Failure::InvalidScene));
    }
    assert_eq!(
        Failure::InvalidScene.json(),
        "{\"error\":{\"kind\":\"invalid_scene\",\"exit_code\":1}}"
    );
    assert_eq!(Failure::Usage.code(), 2);
}

#[test]
fn input_read_failure_is_not_an_admission_or_success() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("private-path"))
        }
    }
    assert_eq!(report(&args(), Broken), Err(Failure::InputRead));
}

#[test]
fn output_write_and_flush_failures_are_reported_without_panics() {
    struct Broken(bool);
    impl Write for Broken {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.0 {
                Ok(bytes.len())
            } else {
                Err(io::Error::other("private-path"))
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("private-path"))
        }
    }
    for writer in [Broken(false), Broken(true)] {
        assert_eq!(emit(writer, "{}"), Err(Failure::OutputWrite));
    }
    let mut bytes = Vec::new();
    emit(&mut bytes, "{}").unwrap();
    assert_eq!(bytes, b"{}\n");
    assert_eq!(
        Failure::OutputWrite.json(),
        "{\"error\":{\"kind\":\"output_write\",\"exit_code\":1}}"
    );
}
