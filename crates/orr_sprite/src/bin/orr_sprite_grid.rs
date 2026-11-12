//! Stdout-only regular-grid document generator; no image or asset I/O.
use orr_sprite::{
    grid::{generate, GridClip, GridSpec},
    Atlas, PlaybackMode, MAX_IMAGE_NAME_BYTES,
};
use std::{ffi::OsString, io::Write, process::ExitCode};

const HELP: &str = "Usage: orr_sprite_grid --image NAME --atlas-width N --atlas-height N
  --columns N --rows N --cell-width N --cell-height N
  --clip NAME --frame-count N --duration-ms N --mode loop|once
  [--offset-x N] [--offset-y N] [--spacing-x N] [--spacing-y N]
  [--first-region-id N] [--first-cell N]

Generate a NEW version-1 sprite JSON document on stdout (with a final newline).
Optional numeric flags default to zero. Indices are zero-based, row-major;
coordinates are top-left pixels. Spacing is between cells. No files are read or
written. --image is inert metadata, not a path to open. This does not re-slice or
preserve IDs in existing bound assets. All flags take one value; no duplicates.
Use --help alone for this text.";

const FLAGS: [&str; 17] = [
    "--image",
    "--atlas-width",
    "--atlas-height",
    "--columns",
    "--rows",
    "--cell-width",
    "--cell-height",
    "--clip",
    "--frame-count",
    "--duration-ms",
    "--mode",
    "--offset-x",
    "--offset-y",
    "--spacing-x",
    "--spacing-y",
    "--first-region-id",
    "--first-cell",
];

fn utf8(arg: OsString) -> Result<String, String> {
    arg.into_string()
        .map_err(|_| "arguments must be valid UTF-8".into())
}

fn run(mut args: impl Iterator<Item = OsString>) -> Result<String, String> {
    let mut values: [Option<String>; FLAGS.len()] = std::array::from_fn(|_| None);
    let mut seen = 0;
    while let Some(arg) = args.next() {
        let flag = utf8(arg)?;
        if flag == "--help" {
            if seen == 0 && args.next().is_none() {
                return Ok(format!("{HELP}\n"));
            }
            return Err("--help must be used alone".into());
        }
        let index = FLAGS
            .iter()
            .position(|&f| f == flag)
            .ok_or("unknown argument")?;
        if values[index].is_some() {
            return Err(format!("duplicate {flag}"));
        }
        let value = utf8(
            args.next()
                .ok_or_else(|| format!("missing value for {flag}"))?,
        )?;
        if value.len() > MAX_IMAGE_NAME_BYTES {
            return Err(format!("value for {flag} exceeds byte limit"));
        }
        values[index] = Some(value);
        seen += 1;
    }
    let value = |i: usize| -> Result<&str, String> {
        values[i]
            .as_deref()
            .or(if i >= 11 { Some("0") } else { None })
            .ok_or_else(|| format!("missing {}", FLAGS[i]))
    };
    let number = |i: usize| -> Result<u64, String> {
        let v = value(i)?;
        if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("{} requires unsigned decimal digits", FLAGS[i]));
        }
        v.parse().map_err(|_| format!("{} overflows u64", FLAGS[i]))
    };
    let u32_number = |i: usize| -> Result<u32, String> {
        u32::try_from(number(i)?).map_err(|_| format!("{} overflows u32", FLAGS[i]))
    };
    let atlas = Atlas {
        image: value(0)?.into(),
        width: u32_number(1)?,
        height: u32_number(2)?,
    };
    let grid = GridSpec {
        columns: u32_number(3)?,
        rows: u32_number(4)?,
        cell_width: u32_number(5)?,
        cell_height: u32_number(6)?,
        offset_x: u32_number(11)?,
        offset_y: u32_number(12)?,
        spacing_x: u32_number(13)?,
        spacing_y: u32_number(14)?,
        first_region_id: u32_number(15)?,
    };
    let clip = GridClip {
        name: value(7)?.into(),
        first_cell: u32_number(16)?,
        frame_count: u32_number(8)?,
        duration_ms: number(9)?,
        mode: match value(10)? {
            "loop" => PlaybackMode::Loop,
            "once" => PlaybackMode::Once,
            _ => return Err("--mode requires loop or once".into()),
        },
    };
    let document = generate(atlas, grid, clip).map_err(|e| e.to_string())?;
    let json = document.to_json().map_err(|e| e.to_string())?;
    Ok(format!("{json}\n"))
}

fn main() -> ExitCode {
    match run(std::env::args_os().skip(1)) {
        Ok(output) => match std::io::stdout().lock().write_all(output.as_bytes()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("orr_sprite_grid: stdout: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("orr_sprite_grid: {error}\nUse --help for usage");
            ExitCode::from(2)
        }
    }
}
