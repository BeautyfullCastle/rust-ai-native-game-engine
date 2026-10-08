//! Explicit, portable project identity for local progress namespaces.
//!
//! Parsing never repairs or generates an identity. Callers must validate metadata
//! before using its identity, and supply a fresh identity for an explicit fork.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectProgress {
    pub schema: u32,
    pub game_id: String,
    pub profile: ProgressProfile,
}

/// Closed inventory of supported progress formats, not a capability grant.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProgressProfile {
    #[serde(rename = "collect-dodge-highscore-v1")]
    CollectDodgeHighscoreV1,
}

impl ProjectProgress {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 {
            return Err("progress.schema must be 1".into());
        }
        self.game_id_bytes()?;
        Ok(())
    }

    /// Decode only canonical lowercase, hyphenated, non-nil UUIDv4 identities.
    pub fn game_id_bytes(&self) -> Result<[u8; 16], String> {
        let source = self.game_id.as_bytes();
        if source.len() != 36 {
            return Err("progress.game_id must be a canonical lowercase UUIDv4".into());
        }
        let mut bytes = [0; 16];
        let mut digit = 0;
        for (position, &value) in source.iter().enumerate() {
            if matches!(position, 8 | 13 | 18 | 23) {
                if value != b'-' {
                    return Err("progress.game_id has invalid UUID hyphens".into());
                }
                continue;
            }
            let nibble = match value {
                b'0'..=b'9' => value - b'0',
                b'a'..=b'f' => value - b'a' + 10,
                _ => return Err("progress.game_id must use lowercase hexadecimal digits".into()),
            };
            bytes[digit / 2] = (bytes[digit / 2] << 4) | nibble;
            digit += 1;
        }
        if bytes == [0; 16] {
            return Err("progress.game_id must not be nil".into());
        }
        if bytes[6] >> 4 != 4 {
            return Err("progress.game_id must be UUID version 4".into());
        }
        if bytes[8] & 0xc0 != 0x80 {
            return Err("progress.game_id must use the UUID RFC variant".into());
        }
        Ok(bytes)
    }

    /// Create fork metadata without changing this project or touching storage.
    /// The caller supplies a new UUID; this operation never generates one.
    pub fn fork_with_game_id(&self, new_id: String) -> Result<Self, String> {
        self.validate()?;
        let fork = Self {
            game_id: new_id,
            ..self.clone()
        };
        fork.validate()?;
        if fork.game_id == self.game_id {
            return Err("a project fork requires a different progress.game_id".into());
        }
        Ok(fork)
    }
}

/// Render bytes canonically without changing their version or variant bits.
/// Formatting alone does not imply that the supplied bytes are a valid UUIDv4.
pub fn format_game_id(bytes: [u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(36);
    for (index, byte) in bytes.into_iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            result.push('-');
        }
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01234567-89ab-4cde-8fab-0123456789ab";
    const OTHER_ID: &str = "01234567-89ab-4cde-bfab-0123456789ac";

    fn progress() -> ProjectProgress {
        ProjectProgress {
            schema: 1,
            game_id: ID.into(),
            profile: ProgressProfile::CollectDodgeHighscoreV1,
        }
    }

    #[test]
    fn canonical_roundtrip() {
        let value = progress();
        value.validate().unwrap();
        assert_eq!(format_game_id(value.game_id_bytes().unwrap()), ID);
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            json,
            format!(r#"{{"schema":1,"game_id":"{ID}","profile":"collect-dodge-highscore-v1"}}"#)
        );
        assert_eq!(
            serde_json::from_str::<ProjectProgress>(&json).unwrap(),
            value
        );
        for variant in [0x80, 0x90, 0xa0, 0xb0] {
            let mut bytes = value.game_id_bytes().unwrap();
            bytes[8] = variant;
            let candidate = ProjectProgress {
                game_id: format_game_id(bytes),
                ..value.clone()
            };
            assert_eq!(candidate.game_id_bytes().unwrap(), bytes);
        }
    }

    #[test]
    fn rejects_duplicate_unknown_missing_null_and_invalid_types() {
        let valid = serde_json::to_value(progress()).unwrap();
        for field in ["schema", "game_id", "profile"] {
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<ProjectProgress>(missing).is_err());
            let mut null = valid.clone();
            null[field] = serde_json::Value::Null;
            assert!(serde_json::from_value::<ProjectProgress>(null).is_err());
            let duplicate = format!(
                "{{\"{field}\":{},{}",
                valid[field],
                &serde_json::to_string(&valid).unwrap()[1..]
            );
            assert!(serde_json::from_str::<ProjectProgress>(&duplicate).is_err());
        }
        let mut unknown = valid.clone();
        unknown["path"] = "../other".into();
        assert!(serde_json::from_value::<ProjectProgress>(unknown).is_err());
        for schema in [
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::json!("1"),
        ] {
            let mut wrong_type = valid.clone();
            wrong_type["schema"] = schema;
            assert!(serde_json::from_value::<ProjectProgress>(wrong_type).is_err());
        }
        assert!(serde_json::from_str::<ProjectProgress>("null").is_err());
    }

    #[test]
    fn rejects_unsupported_schema_and_profiles() {
        for schema in [0, 2, u32::MAX] {
            let value = ProjectProgress {
                schema,
                ..progress()
            };
            assert!(value.validate().is_err());
        }
        for profile in [
            "",
            "collect-dodge-highscore-v2",
            "CollectDodgeHighscoreV1",
            "other",
        ] {
            let mut value = serde_json::to_value(progress()).unwrap();
            value["profile"] = profile.into();
            assert!(serde_json::from_value::<ProjectProgress>(value).is_err());
        }
    }

    #[test]
    fn rejects_noncanonical_and_unsafe_identities() {
        for id in [
            "",
            "0123456789ab4cde8fab0123456789ab",
            "01234567-89AB-4CDE-8FAB-0123456789AB",
            "01234567_89ab-4cde-8fab-0123456789ab",
            "01234567-89ab-5cde-8fab-0123456789ab",
            "01234567-89ab-4cde-7fab-0123456789ab",
            "01234567-89ab-4cde-cfab-0123456789ab",
            "00000000-0000-0000-0000-000000000000",
            "{01234567-89ab-4cde-8fab-0123456789ab}",
            " 01234567-89ab-4cde-8fab-0123456789ab",
            "01234567-89ab-4cde-8fab-0123456789ab\n",
            "../../../../../../../../../../secret",
            "01234567-89ab-4cde-8fab-0123456789/a",
            "01234567-89ab-4cde-8fab-0123456789\\a",
            "01234567-89ab-4cde-8fab-0123456789\0a",
            "01234567-89ab-4cde-8fab-0123456789é",
        ] {
            let value = ProjectProgress {
                game_id: id.into(),
                ..progress()
            };
            assert!(value.validate().is_err(), "accepted {id:?}");
            assert!(value.game_id_bytes().is_err(), "decoded {id:?}");
        }
    }

    #[test]
    fn fork_is_explicit_distinct_validated_and_pure() {
        let original = progress();
        let fork = original.fork_with_game_id(OTHER_ID.into()).unwrap();
        assert_eq!(original, progress());
        assert_eq!(fork.game_id, OTHER_ID);
        assert_eq!(fork.schema, original.schema);
        assert_eq!(fork.profile, original.profile);
        assert!(original.fork_with_game_id(ID.into()).is_err());
        assert!(original.fork_with_game_id("../other".into()).is_err());
        let invalid = ProjectProgress {
            schema: 2,
            ..original
        };
        assert!(invalid.fork_with_game_id(OTHER_ID.into()).is_err());
    }
}
