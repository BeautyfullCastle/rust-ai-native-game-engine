//! Native-only admission journal. This is not an `.orrp` consumed-input replay.
use crate::defense_app::{session, DefenseSession};
use orr_games::coop_defense_game as game;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;

pub const MAX_ADVANCES: u32 = 36_000;
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    /// None means the exact executable's default configuration.
    pub scene: Option<String>,
}
impl Origin {
    pub fn config(&self) -> Result<game::DefenseConfig, String> {
        match &self.scene {
            None => Ok(game::DefenseConfig::default()),
            Some(text) if text.len() <= 65_536 => orr_games::coop_defense_scene::parse_config(text),
            Some(_) => Err("saved scene exceeds 64 KiB".into()),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    ordinal: u32,
    action: u32,
    source_cell: u32,
    target_cell: u32,
    sequence: u32,
}
impl Admission {
    fn input(&self) -> game::DefenseInput {
        game::DefenseInput {
            action: self.action,
            source_cell: self.source_cell,
            target_cell: self.target_cell,
            sequence: self.sequence,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    format: String,
    version: u32,
    executable_sha256: String,
    scenario_build: u64,
    session_build: u64,
    profile: String,
    origin: Origin,
    initial_checksum: u64,
    advances: u32,
    admissions: Vec<Admission>,
    high_water: u32,
    final_checksum: u64,
    payload_sha256: String,
}
#[derive(Clone, Debug)]
pub struct Journal {
    origin: Origin,
    initial_checksum: u64,
    advances: u32,
    admissions: Vec<Admission>,
    exhausted: bool,
}
impl Journal {
    pub fn new(origin: Origin, session: &DefenseSession) -> Self {
        Self {
            origin,
            initial_checksum: session.predicted_frame().checksum(),
            advances: 0,
            admissions: Vec::new(),
            exhausted: false,
        }
    }
    pub fn can_save(&self) -> bool {
        !self.exhausted
    }
    pub fn record(&mut self, input: game::DefenseInput) {
        if self.advances == MAX_ADVANCES {
            self.exhausted = true;
            return;
        }
        self.advances += 1;
        if input != game::DefenseInput::default() {
            self.admissions.push(Admission {
                ordinal: self.advances,
                action: input.action,
                source_cell: input.source_cell,
                target_cell: input.target_cell,
                sequence: input.sequence,
            });
        }
    }
    pub fn encode(
        &mut self,
        session: &DefenseSession,
        high_water: u32,
        identity: &str,
    ) -> Result<Vec<u8>, String> {
        if !self.can_save() {
            return Err("Recording limit reached; gameplay continues, new saves disabled".into());
        }
        if session.predicted_frame().tick() != u64::from(self.advances) {
            return Err("journal/session tick mismatch".into());
        }
        let mut envelope = Envelope {
            format: "stormglass-native-admissions".into(),
            version: 1,
            executable_sha256: identity.into(),
            scenario_build: game::SCENARIO_BUILD_ID,
            session_build: session.build_hash(),
            profile: "local-one-slot0-60hz-delay2".into(),
            origin: self.origin.clone(),
            initial_checksum: self.initial_checksum,
            advances: self.advances,
            admissions: self.admissions.clone(),
            high_water,
            final_checksum: session.predicted_frame().checksum(),
            payload_sha256: String::new(),
        };
        envelope.payload_sha256 = payload_digest(&envelope)?;
        validate(&envelope, identity)?;
        let bytes = serde_json::to_vec(&envelope).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            self.exhausted = true;
            return Err("save exceeds 4 MiB; new saves disabled".into());
        }
        Ok(bytes)
    }
}
// Integrity only, not authentication: anyone editing a save can recompute this.
// Hash canonical struct serialization with this digest field empty, covering the
// future admission tail that cannot yet affect the current Frame checksum.
fn payload_digest(envelope: &Envelope) -> Result<String, String> {
    let mut payload = envelope.clone();
    payload.payload_sha256.clear();
    Ok(hex(&Sha256::digest(
        serde_json::to_vec(&payload).map_err(|e| e.to_string())?,
    )))
}
#[cfg(test)]
pub(crate) fn resign_test_payload(bytes: &[u8]) -> Vec<u8> {
    let mut envelope: Envelope = serde_json::from_slice(bytes).unwrap();
    envelope.payload_sha256 = payload_digest(&envelope).unwrap();
    serde_json::to_vec(&envelope).unwrap()
}
fn validate(e: &Envelope, identity: &str) -> Result<(), String> {
    if payload_digest(e)? != e.payload_sha256 {
        return Err("saved payload integrity mismatch".into());
    }
    if e.format != "stormglass-native-admissions"
        || e.version != 1
        || e.profile != "local-one-slot0-60hz-delay2"
        || e.scenario_build != game::SCENARIO_BUILD_ID
    {
        return Err("unsupported native save format/build/profile".into());
    }
    if identity.len() != 64
        || !identity.bytes().all(|b| b.is_ascii_hexdigit())
        || e.executable_sha256 != identity
    {
        return Err("save requires the exact executable that created it".into());
    }
    if e.advances > MAX_ADVANCES || e.admissions.len() > MAX_ADVANCES as usize {
        return Err("save exceeds admission limit".into());
    }
    let (mut ordinal, mut sequence) = (0, 0);
    for a in &e.admissions {
        if a.ordinal <= ordinal
            || a.ordinal > e.advances
            || !(game::SUMMON..=game::RESTART).contains(&a.action)
            || a.source_cell >= game::CELLS_PER_BOARD
            || a.target_cell >= game::CELLS_PER_BOARD
            || a.sequence <= sequence
            || a.sequence > e.high_water
            || (a.action != game::MOVE_OR_MERGE && (a.source_cell != 0 || a.target_cell != 0))
        {
            return Err("invalid admission ordinal/action/cell/sequence".into());
        }
        ordinal = a.ordinal;
        sequence = a.sequence;
    }
    Ok(())
}
/// Detached bounded reconstruction; dropping this cancels without changing live state.
pub struct Candidate {
    envelope: Envelope,
    session: DefenseSession,
    next: u32,
    entry: usize,
    verified: bool,
}
impl Candidate {
    pub fn parse(bytes: &[u8], identity: &str) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("save exceeds 4 MiB".into());
        }
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|e| {
            format!(
                "invalid save: {:?} at line {}, column {}",
                e.classify(),
                e.line(),
                e.column()
            )
        })?;
        validate(&envelope, identity)?;
        let session = session(envelope.origin.config()?);
        if session.build_hash() != envelope.session_build
            || session.predicted_frame().checksum() != envelope.initial_checksum
        {
            return Err("saved origin/build checksum mismatch".into());
        }
        Ok(Self {
            envelope,
            session,
            next: 0,
            entry: 0,
            verified: false,
        })
    }
    pub fn step(&mut self) -> Result<bool, String> {
        for _ in 0..120 {
            if self.next == self.envelope.advances {
                break;
            }
            self.next += 1;
            let input = if self
                .envelope
                .admissions
                .get(self.entry)
                .is_some_and(|a| a.ordinal == self.next)
            {
                let input = self.envelope.admissions[self.entry].input();
                self.entry += 1;
                input
            } else {
                game::DefenseInput::default()
            };
            self.session.advance(input, vec![]);
            if self.session.predicted_frame().tick() != u64::from(self.next)
                || self.session.head_tick() != u64::from(self.next)
                || self.session.next_send_tick() != u64::from(self.next) + 3
                || self.session.verified_tick() != u64::from(self.next.saturating_sub(1))
            {
                return Err("native reconstruction did not advance".into());
            }
        }
        if self.next != self.envelope.advances {
            return Ok(false);
        }
        if self.session.predicted_frame().checksum() != self.envelope.final_checksum
            || game::run(self.session.predicted_frame()).last_sequence > self.envelope.high_water
        {
            return Err("saved final checksum/sequence mismatch".into());
        }
        self.verified = true;
        Ok(true)
    }
    pub fn finish(self) -> Result<(DefenseSession, Journal, u32), String> {
        if !self.verified {
            return Err("reconstruction is not verified".into());
        }
        let e = self.envelope;
        Ok((
            self.session,
            Journal {
                origin: e.origin,
                initial_checksum: e.initial_checksum,
                advances: e.advances,
                admissions: e.admissions,
                exhausted: false,
            },
            e.high_water,
        ))
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn executable_identity() -> Result<String, String> {
    // Hash the executable once per application launch, not each save.
    let mut file = std::fs::File::open(std::env::current_exe().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut chunk = [0; 65_536];
    loop {
        let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&chunk[..n]);
    }
    Ok(hex(&hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity() -> String {
        "a".repeat(64)
    }
    fn command(sequence: u32) -> game::DefenseInput {
        game::DefenseInput {
            action: game::SUMMON,
            sequence,
            ..Default::default()
        }
    }
    fn roundtrip(n: u32) {
        let mut original = session(game::DefenseConfig::default());
        let mut journal = Journal::new(Origin { scene: None }, &original);
        for ordinal in 1..=n {
            let input = if ordinal + 2 >= n {
                command(ordinal)
            } else {
                game::DefenseInput::default()
            };
            original.advance(input, vec![]);
            journal.record(input);
        }
        let bytes = journal.encode(&original, n + 20, &identity()).unwrap();
        let mut candidate = Candidate::parse(&bytes, &identity()).unwrap();
        while !candidate.step().unwrap() {}
        let (mut restored, _, high_water) = candidate.finish().unwrap();
        assert_eq!(high_water, n + 20);
        assert_eq!(restored.head_tick(), u64::from(n));
        assert_eq!(restored.next_send_tick(), u64::from(n) + 3);
        assert_eq!(restored.verified_tick(), u64::from(n.saturating_sub(1)));
        assert_eq!(
            original.predicted_frame().checksum(),
            restored.predicted_frame().checksum()
        );
        for ordinal in 0..90 {
            let input = if ordinal == 0 {
                command(high_water + 1)
            } else {
                game::DefenseInput::default()
            };
            original.advance(input, vec![]);
            restored.advance(input, vec![]);
            assert_eq!(
                original.predicted_frame().checksum(),
                restored.predicted_frame().checksum(),
                "n={n}, next={ordinal}"
            );
        }
    }
    #[test]
    fn reconstructs_tick_boundaries_and_two_hidden_future_admissions() {
        for n in [0, 1, 2, 125, 400] {
            roundtrip(n);
        }
    }
    #[test]
    fn strict_envelope_and_pending_tail_validation() {
        let mut original = session(game::DefenseConfig::default());
        let mut journal = Journal::new(Origin { scene: None }, &original);
        original.advance(command(1), vec![]);
        journal.record(command(1));
        let bytes = journal.encode(&original, 3, &identity()).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for (field, bad) in [
            ("version", serde_json::json!(2)),
            ("profile", serde_json::json!("web")),
            ("advances", serde_json::json!(36_001)),
            ("high_water", serde_json::json!(0)),
            ("unknown", serde_json::json!(true)),
        ] {
            let mut bad_value = value.clone();
            bad_value[field] = bad;
            let mut bad_bytes = serde_json::to_vec(&bad_value).unwrap();
            if field != "unknown" {
                bad_bytes = resign_test_payload(&bad_bytes);
            }
            assert!(Candidate::parse(&bad_bytes, &identity()).is_err());
        }
        value["admissions"][0]["source_cell"] = serde_json::json!(12);
        assert!(Candidate::parse(
            &resign_test_payload(&serde_json::to_vec(&value).unwrap()),
            &identity()
        )
        .is_err());
        let duplicate =
            String::from_utf8(bytes.clone())
                .unwrap()
                .replacen("{", "{\"version\":1,", 1);
        assert!(Candidate::parse(duplicate.as_bytes(), &identity()).is_err());
        assert!(Candidate::parse(&bytes, &"b".repeat(64)).is_err());
        assert!(Candidate::parse(&vec![b' '; MAX_BYTES + 1], &identity()).is_err());
    }
    #[test]
    fn payload_integrity_covers_both_future_admissions() {
        let mut live = session(game::DefenseConfig::default());
        let mut journal = Journal::new(Origin { scene: None }, &live);
        for seq in 1..=2 {
            let input = command(seq);
            live.advance(input, vec![]);
            journal.record(input);
        }
        let bytes = journal.encode(&live, 2, &identity()).unwrap();
        for entry in 0..2 {
            let mut e: Envelope = serde_json::from_slice(&bytes).unwrap();
            e.admissions[entry].action = game::INVEST;
            assert!(
                matches!(Candidate::parse(&serde_json::to_vec(&e).unwrap(), &identity()), Err(error) if error.contains("integrity"))
            );
        }
        let incomplete = Candidate::parse(&bytes, &identity()).unwrap();
        assert!(incomplete.finish().is_err());
        // Even a recomputed checksum cannot authorize malformed action/sequence.
        let mut e: Envelope = serde_json::from_slice(&bytes).unwrap();
        e.admissions[1].sequence = 1;
        e.payload_sha256 = payload_digest(&e).unwrap();
        assert!(Candidate::parse(&serde_json::to_vec(&e).unwrap(), &identity()).is_err());
    }
    #[test]
    fn malformed_numbers_actions_ordinals_and_nested_fields_are_rejected() {
        let mut live = session(game::DefenseConfig::default());
        let mut journal = Journal::new(Origin { scene: None }, &live);
        for seq in 1..=2 {
            live.advance(command(seq), vec![]);
            journal.record(command(seq));
        }
        let bytes = journal.encode(&live, 2, &identity()).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for (field, bad) in [
            ("ordinal", 0),
            ("ordinal", 3),
            ("action", 0),
            ("action", 6),
            ("source_cell", 12),
            ("target_cell", 12),
            ("sequence", 0),
            ("sequence", 3),
        ] {
            let mut e = value.clone();
            e["admissions"][0][field] = serde_json::json!(bad);
            let bytes = resign_test_payload(&serde_json::to_vec(&e).unwrap());
            assert!(
                matches!(Candidate::parse(&bytes,&identity()),Err(error) if error.contains("admission")),
                "{field}={bad}"
            );
        }
        for bad in [
            serde_json::json!(-1),
            serde_json::json!(4294967296_u64),
            serde_json::json!(1.5),
        ] {
            let mut e = value.clone();
            e["admissions"][0]["sequence"] = bad;
            assert!(Candidate::parse(&serde_json::to_vec(&e).unwrap(), &identity()).is_err());
        }
        let mut duplicate = value.clone();
        duplicate["admissions"][1]["ordinal"] = serde_json::json!(1);
        assert!(Candidate::parse(
            &resign_test_payload(&serde_json::to_vec(&duplicate).unwrap()),
            &identity()
        )
        .is_err());
        let mut unknown = value.clone();
        unknown["origin"]["unknown"] = serde_json::json!(true);
        assert!(Candidate::parse(&serde_json::to_vec(&unknown).unwrap(), &identity()).is_err());
        let mut missing = value;
        missing.as_object_mut().unwrap().remove("advances");
        assert!(Candidate::parse(&serde_json::to_vec(&missing).unwrap(), &identity()).is_err());
        let mut trailing = bytes;
        trailing.extend_from_slice(b" {}");
        assert!(Candidate::parse(&trailing, &identity()).is_err());
    }
    #[test]
    fn recording_limit_does_not_stop_session() {
        let mut original = session(game::DefenseConfig::default());
        let mut journal = Journal::new(Origin { scene: None }, &original);
        // Exercise recorder boundary without spending 36,000 simulation steps.
        journal.advances = MAX_ADVANCES;
        assert!(journal.can_save());
        journal.record(Default::default());
        original.advance(Default::default(), vec![]);
        assert!(!journal.can_save());
        assert_eq!(original.predicted_frame().tick(), 1);
        assert!(journal.encode(&original, 0, &identity()).is_err());
    }
}
