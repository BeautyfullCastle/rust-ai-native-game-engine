//! Command-line options every `orr_app` program understands.

use std::iter::Peekable;
use std::net::SocketAddr;
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
  --host [port|addr]  run a relay server in this process and play on it (default 127.0.0.1:7777;
                      use 0.0.0.0:7777 for other PCs); the game starts when every player joined
  --connect <addr>    play on a relay server (host:port), for example one started with --host
  --slot <n>          with --connect: ask for this slot (to take a slot back with --token)
  --token <n>         with --connect: the token a window printed, to take its slot back
  --help              print this text";

/// Where `--host` listens when no address is given.
pub const DEFAULT_HOST: &str = "127.0.0.1:7777";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    pub screenshot: Option<PathBuf>,
    pub frames: u32,
    pub ticks: Option<u64>,
    pub size: (u32, u32),
    pub vsync: bool,
    pub help: bool,
    /// `--host`: the address text after it (empty for the default).
    pub host: Option<String>,
    /// `--connect`: the server address.
    pub connect: Option<String>,
    pub slot: Option<u8>,
    pub token: u64,
}

/// How a run gets its simulation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunMode {
    /// No server: the app runs the game itself (the default).
    Offline,
    /// A relay server in this process on `bind`, and this window plays on it.
    Host { bind: SocketAddr },
    /// This window plays on the relay server at `addr`.
    Connect { addr: String },
}

impl Default for Args {
    fn default() -> Self {
        Self {
            screenshot: None,
            frames: DEFAULT_FRAMES,
            ticks: None,
            size: DEFAULT_SIZE,
            vsync: true,
            help: false,
            host: None,
            connect: None,
            slot: None,
            token: 0,
        }
    }
}

impl Args {
    pub fn from_env() -> Result<Self, String> {
        Self::parse(std::env::args().skip(1))
    }

    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut out = Self::default();
        let mut args = args.into_iter().peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--screenshot" => out.screenshot = Some(PathBuf::from(value(&mut args, "--screenshot")?)),
                "--frames" => out.frames = whole("--frames", &value(&mut args, "--frames")?)?,
                "--ticks" => out.ticks = Some(whole("--ticks", &value(&mut args, "--ticks")?)?),
                "--size" => out.size = size(&value(&mut args, "--size")?)?,
                "--no-vsync" => out.vsync = false,
                // The address is optional: the next word is one unless it is another option.
                "--host" => out.host = Some(args.next_if(|next| !next.starts_with("--")).unwrap_or_default()),
                "--connect" => out.connect = Some(value(&mut args, "--connect")?),
                "--slot" => out.slot = Some(whole("--slot", &value(&mut args, "--slot")?)?),
                "--token" => out.token = whole("--token", &value(&mut args, "--token")?)?,
                "--help" | "-h" => out.help = true,
                other => return Err(format!("unknown option '{other}'
{USAGE}")),
            }
        }
        if out.host.is_some() && out.connect.is_some() {
            return Err("--host and --connect cannot be used together".to_string());
        }
        if (out.slot.is_some() || out.token != 0) && out.connect.is_none() {
            return Err("--slot and --token need --connect".to_string());
        }
        Ok(out)
    }

    /// Offline, hosting or connecting, as the command line says.
    pub fn mode(&self) -> Result<RunMode, String> {
        match (&self.host, &self.connect) {
            (Some(text), _) => Ok(RunMode::Host { bind: host_address(text)? }),
            (None, Some(addr)) => Ok(RunMode::Connect { addr: addr.clone() }),
            (None, None) => Ok(RunMode::Offline),
        }
    }
}

fn value(args: &mut Peekable<impl Iterator<Item = String>>, name: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{name} needs a value"))
}

/// `""` is the default address, `7777` a port on the loopback address, anything else `addr:port`.
fn host_address(text: &str) -> Result<SocketAddr, String> {
    if text.is_empty() {
        return Ok(DEFAULT_HOST.parse().expect("the default address is valid"));
    }
    if let Ok(port) = text.parse::<u16>() {
        return Ok(SocketAddr::from(([127, 0, 0, 1], port)));
    }
    text.parse().map_err(|_| format!("--host needs a port or addr:port, got '{text}'"))
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
    fn the_run_mode_follows_host_and_connect() {
        assert_eq!(parse("").unwrap().mode().unwrap(), RunMode::Offline);
        assert_eq!(parse("--host").unwrap().mode().unwrap(), RunMode::Host { bind: DEFAULT_HOST.parse().unwrap() });
        assert_eq!(parse("--host 9000").unwrap().mode().unwrap(), RunMode::Host { bind: "127.0.0.1:9000".parse().unwrap() });
        assert_eq!(parse("--host 0.0.0.0:7777").unwrap().mode().unwrap(), RunMode::Host { bind: "0.0.0.0:7777".parse().unwrap() });
        // The address is optional: another option after --host is not read as its value.
        let a = parse("--host --screenshot a.png").unwrap();
        assert_eq!((a.host.as_deref(), a.screenshot), (Some(""), Some(PathBuf::from("a.png"))));
        let a = parse("--connect 127.0.0.1:7777 --slot 1 --token 5").unwrap();
        assert_eq!(a.mode().unwrap(), RunMode::Connect { addr: "127.0.0.1:7777".to_string() });
        assert_eq!((a.slot, a.token), (Some(1), 5));
    }

    #[test]
    fn network_options_must_fit_together() {
        assert!(parse("--host --connect 127.0.0.1:1").unwrap_err().contains("cannot be used together"));
        assert!(parse("--slot 1").unwrap_err().contains("need --connect"));
        assert!(parse("--host --token 3").unwrap_err().contains("need --connect"));
        assert!(parse("--connect").unwrap_err().contains("needs a value"));
        assert!(parse("--host abc").unwrap().mode().unwrap_err().contains("port or addr:port"));
    }

    #[test]
    fn help_is_a_flag_not_an_error() {
        assert!(parse("--help").unwrap().help);
    }
}
