//! Starter launch arguments. Parsing never opens a listener or loads credentials from files or the environment.
use std::net::SocketAddr;

pub const USAGE: &str = "usage: editor <delivery|switchboard> <scene.yaml> | editor --project <registered-project.json> [--erp <loopback-address:port> --erp-token <name:token:caps>]";

pub struct EditorArgs {
    pub source: String,
    pub path: String,
    pub listener: Option<orr_remote::ServerConfig>,
}

impl EditorArgs {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut args = args.into_iter();
        let source = args.next().ok_or(USAGE)?;
        let path = args.next().ok_or(USAGE)?;
        let mut bind: Option<SocketAddr> = None;
        let mut tokens = Vec::new();
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--erp" => {
                    if bind.is_some() {
                        return Err("duplicate --erp".into());
                    }
                    let value = args.next().ok_or("--erp needs an address")?;
                    let address: SocketAddr = value.parse().map_err(|_| "invalid --erp address")?;
                    if !address.ip().is_loopback() {
                        return Err("--erp must bind to loopback".into());
                    }
                    bind = Some(address);
                }
                "--erp-token" => {
                    let value = args.next().ok_or("--erp-token needs a value")?;
                    // Do not echo a malformed token or capability string in diagnostics.
                    tokens.push(
                        orr_remote::TokenEntry::parse(&value).map_err(|_| "invalid --erp-token")?,
                    );
                }
                _ => return Err(USAGE.into()),
            }
        }
        if bind.is_none() && !tokens.is_empty() {
            return Err("authentication options require --erp".into());
        }
        let listener = match bind {
            None => None,
            Some(bind) => {
                if source != "--project" {
                    return Err("--erp requires --project".into());
                }
                let auth = if tokens.is_empty() {
                    return Err("--erp requires explicit authentication".into());
                } else {
                    orr_remote::Auth::Tokens(tokens)
                };
                let mut config = orr_remote::ServerConfig::new(auth);
                config.bind = bind;
                Some(config)
            }
        };
        Ok(Self {
            source,
            path,
            listener,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(values: &[&str]) -> Result<EditorArgs, String> {
        EditorArgs::parse(values.iter().map(|value| (*value).to_owned()))
    }
    #[test]
    fn old_launch_has_no_listener() {
        assert!(parse(&["delivery", "scene.yaml"])
            .unwrap()
            .listener
            .is_none());
        assert!(parse(&["--project", "project.json"])
            .unwrap()
            .listener
            .is_none());
    }
    #[test]
    fn explicit_loopback_only() {
        for address in ["127.0.0.1:0", "[::1]:7777"] {
            let parsed = parse(&[
                "--project",
                "project.json",
                "--erp",
                address,
                "--erp-token",
                "test:fixture:read",
            ])
            .unwrap();
            assert!(matches!(
                parsed.listener.unwrap().auth,
                orr_remote::Auth::Tokens(_)
            ));
        }
        for address in ["0.0.0.0:7777", "[::]:7777", "192.168.1.2:7777"] {
            assert!(parse(&[
                "--project",
                "p",
                "--erp",
                address,
                "--erp-token",
                "test:fixture:read"
            ])
            .is_err());
        }
    }
    #[test]
    fn rejects_ambiguous_or_incomplete_options() {
        for tail in [
            vec!["--erp", "127.0.0.1:0"],
            vec!["--erp-dev"],
            vec!["--erp-token", "test:fixture:read"],
            vec!["--erp"],
            vec!["--unknown"],
            vec![
                "--erp",
                "127.0.0.1:0",
                "--erp-dev",
                "--erp-token",
                "test:fixture:read",
            ],
            vec!["--erp", "127.0.0.1:0", "--erp", "127.0.0.1:1", "--erp-dev"],
        ] {
            let mut args = vec!["--project", "p"];
            args.extend(tail);
            assert!(parse(&args).is_err());
        }
        assert!(parse(&["delivery", "s", "--erp", "127.0.0.1:0", "--erp-dev"]).is_err());
    }
    #[test]
    fn malformed_tokens_and_legacy_listener_are_rejected() {
        for token in [
            "missing",
            ":fixture:read",
            "test::read",
            "test:fixture:unknown",
        ] {
            assert_eq!(
                parse(&["--project", "p", "--erp-token", token])
                    .err()
                    .unwrap(),
                "invalid --erp-token"
            );
        }
        assert!(parse(&["--project", "p", "--erp-token"]).is_err());
        assert_eq!(
            parse(&[
                "delivery",
                "s",
                "--erp",
                "127.0.0.1:0",
                "--erp-token",
                "test:fixture:read"
            ])
            .err()
            .unwrap(),
            "--erp requires --project"
        );
    }
    #[test]
    fn supplied_tokens_and_bind_are_preserved_in_either_order() {
        let parsed = parse(&[
            "--project",
            "p",
            "--erp-token",
            "reader:fixture-one:read",
            "--erp",
            "[::1]:9123",
            "--erp-token",
            "player:fixture-two:sim_control",
        ])
        .unwrap();
        let config = parsed.listener.unwrap();
        assert!(config.listen);
        assert!(config.allowed_origins.is_empty());
        assert_eq!(config.bind, "[::1]:9123".parse::<SocketAddr>().unwrap());
        assert_eq!(
            config.auth,
            orr_remote::Auth::Tokens(vec![
                orr_remote::TokenEntry::parse("reader:fixture-one:read").unwrap(),
                orr_remote::TokenEntry::parse("player:fixture-two:sim_control").unwrap()
            ])
        );
    }
    #[test]
    fn token_caps_preserved_and_errors_redacted() {
        let parsed = parse(&[
            "--project",
            "p",
            "--erp",
            "127.0.0.1:0",
            "--erp-token",
            "test:fixture:read",
        ])
        .unwrap();
        assert_eq!(
            parsed.listener.unwrap().auth,
            orr_remote::Auth::Tokens(vec![
                orr_remote::TokenEntry::parse("test:fixture:read").unwrap()
            ])
        );
        let error = parse(&[
            "--project",
            "p",
            "--erp-token",
            "test:fixture:private-invalid-cap",
        ])
        .err()
        .unwrap();
        assert!(!error.contains("fixture"));
        assert!(!error.contains("private-invalid-cap"));
    }
}
