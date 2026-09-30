//! Command line of the editor binary.

use std::path::PathBuf;

/// Usage text.
pub const USAGE: &str = "\
orr_editor [--scene <file>] [--select <name>] [--play-ticks <n>] [--script <file>]
           [--screenshot <out.png> [--frames <n>]] [--size <WxH>]

  --scene <file>        scene to open (default scenes/physics_demo.scene.yaml)
  --select <name>       select the entity with this name (or GUID) at start
  --play-ticks <n>      start play and run n ticks (paused) at start
  --script <file>       run editor commands (one per line) at start
  --screenshot <png>    render --frames frames, save the window's own
                        framebuffer to the PNG and exit
  --frames <n>          frames before the screenshot (default 30)
  --size <WxH>          window size in points (default 1600x900)
";

/// Parsed flags.
#[derive(Clone, Debug, PartialEq)]
pub struct Args {
    /// `--scene`.
    pub scene: Option<PathBuf>,
    /// `--select`.
    pub select: Option<String>,
    /// `--play-ticks`.
    pub play_ticks: Option<u32>,
    /// `--script`.
    pub script: Option<PathBuf>,
    /// `--screenshot`.
    pub screenshot: Option<PathBuf>,
    /// `--frames`.
    pub frames: u64,
    /// `--size`.
    pub size: (f32, f32),
}

impl Default for Args {
    fn default() -> Self {
        Self { scene: None, select: None, play_ticks: None, script: None, screenshot: None, frames: 30, size: (1600.0, 900.0) }
    }
}

impl Args {
    /// Parses the arguments after the program name. `Err` carries a message
    /// (for `--help` the usage text).
    pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Args, String> {
        let mut out = Args::default();
        let mut it = args.into_iter();
        while let Some(a) = it.next() {
            let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value\n\n{USAGE}"));
            match a.as_str() {
                "--scene" => out.scene = Some(PathBuf::from(value("--scene")?)),
                "--select" => out.select = Some(value("--select")?),
                "--script" => out.script = Some(PathBuf::from(value("--script")?)),
                "--screenshot" => out.screenshot = Some(PathBuf::from(value("--screenshot")?)),
                "--play-ticks" => out.play_ticks = Some(value("--play-ticks")?.parse().map_err(|_| "--play-ticks needs a whole number".to_string())?),
                "--frames" => out.frames = value("--frames")?.parse().map_err(|_| "--frames needs a whole number".to_string())?,
                "--size" => {
                    let v = value("--size")?;
                    let (w, h) = v.split_once('x').ok_or("--size needs WxH, like 1600x900")?;
                    out.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
                }
                "-h" | "--help" => return Err(USAGE.to_string()),
                other => return Err(format!("unknown argument '{other}'\n\n{USAGE}")),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Args, String> {
        Args::parse(s.split_whitespace().map(str::to_string))
    }

    #[test]
    fn parses_flags() {
        let a = parse("--scene a.yaml --screenshot /tmp/x.png --frames 12 --play-ticks 90 --select body_05 --size 800x600").unwrap();
        assert_eq!(a.scene, Some(PathBuf::from("a.yaml")));
        assert_eq!(a.screenshot, Some(PathBuf::from("/tmp/x.png")));
        assert_eq!((a.frames, a.play_ticks, a.select.as_deref(), a.size), (12, Some(90), Some("body_05"), (800.0, 600.0)));
        assert_eq!(parse("").unwrap(), Args::default());
    }

    #[test]
    fn rejects_bad_flags() {
        assert!(parse("--frames x").is_err());
        assert!(parse("--scene").is_err());
        assert!(parse("--nope").is_err());
        assert!(parse("--size 10").is_err());
    }
}
