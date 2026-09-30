//! Capability tokens: who may call what.

use core::fmt;

/// One capability. A method requires exactly one (see [`crate::methods`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cap {
    /// Look at the scene, the registry, the history and the sim state.
    Read,
    /// Change the scene document (edits, transactions, undo/redo, load).
    SceneEdit,
    /// Start, stop and steer the play session. Also needed, together with
    /// `SceneEdit`, to change the state of a running sim (a debug edit).
    SimControl,
    /// Accept a proposal into the document (`proposal.accept`). Kept apart
    /// from `SceneEdit` so a host can let an agent propose and verify while
    /// a person keeps the decision. `all` and dev mode (`--erp-dev`) include it.
    Approve,
}

impl Cap {
    /// The name used in tokens and errors.
    pub fn name(self) -> &'static str {
        match self {
            Cap::Read => "read",
            Cap::SceneEdit => "scene_edit",
            Cap::SimControl => "sim_control",
            Cap::Approve => "approve",
        }
    }
}

impl fmt::Display for Cap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A set of capabilities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Caps(u8);

impl Caps {
    /// No capability.
    pub const NONE: Caps = Caps(0);
    /// Every capability.
    pub const ALL: Caps = Caps(15);

    fn bit(c: Cap) -> u8 {
        match c {
            Cap::Read => 1,
            Cap::SceneEdit => 2,
            Cap::SimControl => 4,
            Cap::Approve => 8,
        }
    }

    /// A set of these capabilities.
    pub fn of(list: &[Cap]) -> Caps {
        Caps(list.iter().fold(0, |a, &c| a | Self::bit(c)))
    }

    /// True if `c` is in the set.
    pub fn has(self, c: Cap) -> bool {
        self.0 & Self::bit(c) != 0
    }

    /// The capabilities in the set, in a fixed order.
    pub fn list(self) -> Vec<Cap> {
        [Cap::Read, Cap::SceneEdit, Cap::SimControl, Cap::Approve].into_iter().filter(|&c| self.has(c)).collect()
    }

    /// Parses `read,scene_edit,sim_control,approve`, or `all`. Names may be
    /// separated by commas or `+`.
    pub fn parse(text: &str) -> Result<Caps, String> {
        let mut caps = Caps::NONE;
        for part in text.split([',', '+']).map(str::trim).filter(|p| !p.is_empty()) {
            match part {
                "all" => caps = Caps::ALL,
                "read" => caps.0 |= Self::bit(Cap::Read),
                "scene_edit" => caps.0 |= Self::bit(Cap::SceneEdit),
                "sim_control" => caps.0 |= Self::bit(Cap::SimControl),
                "approve" => caps.0 |= Self::bit(Cap::Approve),
                other => return Err(format!("unknown capability '{other}' (read, scene_edit, sim_control, approve, all)")),
            }
        }
        Ok(caps)
    }
}

impl fmt::Display for Caps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.list().into_iter().map(Cap::name).collect();
        f.write_str(&if names.is_empty() { "none".to_string() } else { names.join(",") })
    }
}

/// One accepted token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenEntry {
    /// The secret. Compared in constant time.
    pub token: String,
    /// The client name: becomes `Origin::Agent(client)` in the history.
    pub client: String,
    /// What this token allows.
    pub caps: Caps,
}

impl TokenEntry {
    /// Parses `name:token:caps` (the command line form), e.g. `claude:s3cret:read,scene_edit`.
    /// The name may not contain `:`; the token may (the caps are after the last `:`).
    pub fn parse(spec: &str) -> Result<TokenEntry, String> {
        let (client, rest) = spec.split_once(':').ok_or("expected name:token:caps")?;
        let (token, caps) = rest.rsplit_once(':').ok_or("expected name:token:caps")?;
        if client.is_empty() || token.is_empty() {
            return Err("name and token must not be empty".into());
        }
        Ok(TokenEntry { token: token.to_string(), client: client.to_string(), caps: Caps::parse(caps)? })
    }
}

/// How connections are authenticated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    /// Every connection must present one of these tokens (first message
    /// `auth`, or `?token=` in the WebSocket URL).
    Tokens(Vec<TokenEntry>),
    /// Local development: no token, every connection is client `dev` with
    /// every capability. Only allowed on a loopback bind address. There is
    /// deliberately no `Default`: leaving auth out must be a decision.
    DevNoAuth,
}

/// Constant-time (for equal lengths) comparison of two secrets.
pub(crate) fn secret_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_specs() {
        let t = TokenEntry::parse("claude:s3:cret:read,scene_edit").unwrap();
        assert_eq!((t.client.as_str(), t.token.as_str()), ("claude", "s3:cret"));
        assert!(t.caps.has(Cap::Read) && t.caps.has(Cap::SceneEdit) && !t.caps.has(Cap::SimControl));
        assert_eq!(TokenEntry::parse("a:b:all").unwrap().caps, Caps::ALL);
        assert!(TokenEntry::parse("a:b:fly").is_err());
        let a = TokenEntry::parse("a:b:read,scene_edit,approve").unwrap();
        assert!(a.caps.has(Cap::Approve) && !a.caps.has(Cap::SimControl));
        assert!(Caps::ALL.has(Cap::Approve));
        assert_eq!(Caps::parse("read+approve").unwrap().to_string(), "read,approve");
        assert!(TokenEntry::parse("nocolon").is_err());
        assert!(TokenEntry::parse(":b:read").is_err());
    }
}
