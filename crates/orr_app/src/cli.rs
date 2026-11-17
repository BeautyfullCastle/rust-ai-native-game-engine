//! Command-line options every `orr_app` program understands.

use std::path::PathBuf;

/// Frames drawn before a screenshot when `--frames` is not given.
pub const DEFAULT_FRAMES: u32 = 30;
/// Window and screenshot size when `--size` is not given.
pub const DEFAULT_SIZE: (u32, u32) = (900, 900);

pub const USAGE: &str = "options:
  --screenshot <png>  draw --frames frames, save one picture, then quit
  --frames <n>        frames drawn before the picture (default 30)
  --ticks <n>         run the sim to tick n, then hold it there
  --size <WxH>        window size, and the size of the picture (default 900x900)
  --no-vsync          do not wait for the display
  --help              print this text";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    pub screenshot: Option<PathBuf>,
    pub frames: u32,
    pub ticks: Option<u64>,
    pub size: (u32, u32),
    pub vsync: bool,
    pub help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self { screenshot: None, frames: DEFAULT_FRAMES, ticks: None, size: DEFAULT_SIZE, vsync: true, help: false }
    }
}

impl Args {
    pub fn from_env() -> Result<Self, String> {
        Self::parse(std::env::args().skip(1))
    }

    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut out = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
            match arg.as_str() {
                "--screenshot" => out.screenshot = Some(PathBuf::from(value("--screenshot")?)),
                "--frames" => out.frames = whole("--frames", &value("--frames")?)?,
                "--ticks" => out.ticks = Some(whole("--ticks", &value("--ticks")?)?),
                "--size" => out.size = size(&value("--size")?)?,
                "--no-vsync" => out.vsync = false,
                "--help" | "-h" => out.help = true,
                other => return Err(format!("unknown option '{other}'\n{USAGE}")),
            }
        }
        Ok(out)
    }
}

fn whole<T: std::str::FromStr>(name: &str, text: &str) -> Result<T, String> {
    text.parse().map_err(|_| format!("{name} needs a whole number, got '{text}'"))
}

fn size(text: &str) -> Result<(u32, u32), String> {
    let bad = || format!("--size needs WIDTHxHEIGHT, got '{text}'");
    let (w, h) = text.split_once('x').ok_or_else(bad)?;
    let (w, h): (u32, u32) = (w.parse().map_err(|_| bad())?, h.parse().map_err(|_| bad())?);
    if w == 0 || h == 0 {
        return Err(bad());
    }
    Ok((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Args, String> {
        Args::parse(line.split_whitespace().map(String::from))
    }

    #[test]
    fn no_arguments_give_the_defaults() {
        assert_eq!(parse("").unwrap(), Args::default());
    }

    #[test]
    fn every_option_is_read() {
        let a = parse("--screenshot out.png --frames 12 --ticks 90 --size 640x480 --no-vsync").unwrap();
        assert_eq!(a.screenshot, Some(PathBuf::from("out.png")));
        assert_eq!((a.frames, a.ticks, a.size, a.vsync), (12, Some(90), (640, 480), false));
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(parse("--frames").unwrap_err().contains("needs a value"));
        assert!(parse("--frames x").unwrap_err().contains("whole number"));
        assert!(parse("--ticks -1").is_err());
        assert!(parse("--size 640").is_err());
        assert!(parse("--size 0x10").is_err());
        assert!(parse("--bogus").unwrap_err().contains("unknown option"));
    }

    #[test]
    fn help_is_a_flag_not_an_error() {
        assert!(parse("--help").unwrap().help);
    }
}
