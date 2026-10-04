//! Command line of the editor binary.

use std::path::PathBuf;

use crate::game::EditorGame;

/// Usage text.
pub const USAGE: &str = "\
orr_editor [--game physics|arena] [--scene <file>] [--select <name>] [--play-ticks <n>] [--script <file>]
           [--screenshot <out.png> [--frames <n>] [--screenshot-settle]] [--size <WxH>]
           [--erp <addr>] [--erp-token <name:token:caps>]... [--erp-dev]
orr_editor --connect <ws://host:port> [--token <t>] [same flags, but --scene/--erp]

The editor is a view. The simulation and the scene document live in a
host: by default a thread of this process, started on the scene and
connected in-process; with --connect, an `orr_remote_host` (or another
editor's ERP) that is already running.

  --scene <file>        scene to open (default scene depends on --game)
  --game <name>         local game: physics (default) or arena; cannot be used with --connect
  --connect <url>       attach to a running host instead of starting one: the
                        editor then shows and edits THAT host's scene and play
                        session (with --token when it needs one; a dev-mode
                        host needs none)
  --token <t>           the token for --connect
  --select <name>       select the entity with this name (or GUID) at start
  --play-ticks <n>      start play and run n ticks (paused) at start
  --script <file>       run editor commands (one per line) at start
  --screenshot <png>    render --frames frames, save the window's own
                        framebuffer to the PNG and exit
  --frames <n>          frames before the screenshot (default 30)
  --screenshot-settle   additionally wait for a current paused frame, refreshed
                        panels and expired agent pulses (10 second deadline)
  --size <WxH>          window size in points (default 1600x900)
  --erp <addr>          serve ERP (JSON-RPC over WebSocket) on this address,
                        e.g. 127.0.0.1:7777, so AI agents can edit and play
                        the open scene (same undo history as the window)
  --erp-token <n:t:c>   a client token: name:token:caps, caps = read,
                        scene_edit, sim_control (comma separated) or all
  --erp-dev             no tokens, every client has every capability
                        (loopback addresses only)
";

/// Parsed flags.
#[derive(Clone, Debug, PartialEq)]
pub struct Args {
    /// Optional local game selection. `None` preserves the PhysGame default.
    pub game: Option<EditorGame>,
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
    /// `--screenshot-settle` (requires `--screenshot`).
    pub screenshot_settle: bool,
    /// `--size`.
    pub size: (f32, f32),
    /// `--erp`.
    pub erp: Option<std::net::SocketAddr>,
    /// `--erp-token`, as given.
    pub erp_tokens: Vec<String>,
    /// `--erp-dev`.
    pub erp_dev: bool,
    /// `--connect`.
    pub connect: Option<String>,
    /// `--token`.
    pub token: Option<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self { game: None, scene: None, select: None, play_ticks: None, script: None, screenshot: None, frames: 30, screenshot_settle: false, size: (1600.0, 900.0), erp: None, erp_tokens: Vec::new(), erp_dev: false, connect: None, token: None }
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
                "--game" => out.game = Some(EditorGame::from_local_name(&value("--game")?)?),
                "--scene" => out.scene = Some(PathBuf::from(value("--scene")?)),
                "--select" => out.select = Some(value("--select")?),
                "--script" => out.script = Some(PathBuf::from(value("--script")?)),
                "--screenshot" => out.screenshot = Some(PathBuf::from(value("--screenshot")?)),
                "--screenshot-settle" => out.screenshot_settle = true,
                "--play-ticks" => out.play_ticks = Some(value("--play-ticks")?.parse().map_err(|_| "--play-ticks needs a whole number".to_string())?),
                "--frames" => out.frames = value("--frames")?.parse().map_err(|_| "--frames needs a whole number".to_string())?,
                "--size" => {
                    let v = value("--size")?;
                    let (w, h) = v.split_once('x').ok_or("--size needs WxH, like 1600x900")?;
                    out.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
                }
                "--erp" => out.erp = Some(value("--erp")?.parse().map_err(|e| format!("--erp: {e}"))?),
                "--erp-token" => out.erp_tokens.push(value("--erp-token")?),
                "--erp-dev" => out.erp_dev = true,
                "--connect" => out.connect = Some(value("--connect")?),
                "--token" => out.token = Some(value("--token")?),
                "-h" | "--help" => return Err(USAGE.to_string()),
                other => return Err(format!("unknown argument '{other}'\n\n{USAGE}")),
            }
        }
        if out.connect.is_some() && (out.scene.is_some() || out.erp.is_some()) {
            return Err("--connect attaches to a host that has its own scene: it cannot be combined with --scene or --erp".to_string());
        }
        if out.connect.is_some() && out.game.is_some() {
            return Err("--game selects a local game and cannot be combined with --connect".to_string());
        }
        if out.token.is_some() && out.connect.is_none() {
            return Err("--token is for --connect (use --erp-token to set tokens for --erp)".to_string());
        }
        if out.screenshot_settle && out.screenshot.is_none() {
            return Err("--screenshot-settle requires --screenshot".to_string());
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
        assert!(parse("--connect ws://h:1 --scene a.yaml").is_err());
        assert!(parse("--token x").is_err());
        assert!(parse("--screenshot-settle").is_err());
    }

    #[test]
    fn parses_connect() {
        let a = parse("--connect ws://127.0.0.1:7790 --token s3 --screenshot /tmp/a.png").unwrap();
        assert_eq!((a.connect.as_deref(), a.token.as_deref()), (Some("ws://127.0.0.1:7790"), Some("s3")));
    }

    #[test]
    fn local_game_defaults_to_physics_and_accepts_arena() {
        assert_eq!(parse("").unwrap().game, None);
        assert_eq!(parse("--game physics").unwrap().game, Some(EditorGame::PhysGame));
        assert_eq!(parse("--game arena").unwrap().game, Some(EditorGame::Arena));
    }

    #[test]
    fn local_game_rejects_unknown_and_remote_selection() {
        assert!(parse("--game nope").is_err());
        assert!(parse("--game arena --connect ws://127.0.0.1:7790").is_err());
    }

    #[test]
    fn screenshot_settle_is_opt_in() {
        assert!(!parse("--screenshot out.png --frames 30").unwrap().screenshot_settle);
        assert!(parse("--screenshot out.png --frames 30 --screenshot-settle").unwrap().screenshot_settle);
    }
}
