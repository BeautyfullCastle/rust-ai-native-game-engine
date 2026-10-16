//! Generation-fenced P2P control messages. Version 3 wraps a complete v2
//! payload using the same ORRQ/ORRJ/ORRB magic and little-endian integers:
//! magic, version u32, join_id u64, attempt u32, player_count u8, joiner u8,
//! donor u8, active_count u32, sorted active slots u8[], payload_len u32,
//! payload, xxh3 u64 over all preceding bytes.
//!
//! The active roster plus joiner determines vacant slots. Checksums detect
//! corruption, not forgery. Callers must verify transport senders and provide a
//! completed membership view, assigning a fresh nonzero id to each generation.
//! Input packets/backlogs still require caller-controlled link scoping. These
//! adapters do not discover peers, authenticate, or clean up remote holds.

use crate::{join, Game, InputSource, JoinError, JoinRoster, JoinTicket, PlayerSlot, Session};

const VERSION: u32 = 3;

/// Failure before accepting a checked control message.
#[derive(Debug)]
pub enum CheckedJoinError {
    Join(JoinError),
    InvalidContext,
    GenerationMismatch { expected: u64, got: u64 },
    ManifestMismatch,
    InnerMismatch,
    WireLimit { limit: usize },
    MissingTicket,
    WrongDonor(PlayerSlot),
}
impl core::fmt::Display for CheckedJoinError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Join(e) => write!(f, "{e}"),
            Self::InvalidContext => {
                write!(f, "checked join requires nonzero generation and attempt")
            }
            Self::GenerationMismatch { expected, got } => {
                write!(f, "join generation {got} differs from {expected}")
            }
            Self::ManifestMismatch => write!(f, "checked join membership differs"),
            Self::InnerMismatch => write!(f, "inner join payload differs from checked context"),
            Self::WireLimit { limit } => write!(f, "snapshot wire byte limit {limit} exceeded"),
            Self::MissingTicket => write!(f, "accepted join has no pending ticket"),
            Self::WrongDonor(slot) => {
                write!(f, "slot {} is not the expected snapshot donor", slot.0)
            }
        }
    }
}
impl std::error::Error for CheckedJoinError {}
impl From<JoinError> for CheckedJoinError {
    fn from(value: JoinError) -> Self {
        Self::Join(value)
    }
}

/// Immutable expected generation, attempt and complete slot manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedJoinContext {
    join_id: u64,
    attempt: u32,
    roster: JoinRoster,
}
impl CheckedJoinContext {
    pub fn new(join_id: u64, attempt: u32, roster: JoinRoster) -> Result<Self, CheckedJoinError> {
        if join_id == 0 || attempt == 0 {
            return Err(CheckedJoinError::InvalidContext);
        }
        Ok(Self {
            join_id,
            attempt,
            roster,
        })
    }
    pub fn join_id(&self) -> u64 {
        self.join_id
    }
    pub fn attempt(&self) -> u32 {
        self.attempt
    }
    pub fn roster(&self) -> &JoinRoster {
        &self.roster
    }

    fn check(&self, other: &Self) -> Result<(), CheckedJoinError> {
        if self.join_id != other.join_id {
            return Err(CheckedJoinError::GenerationMismatch {
                expected: self.join_id,
                got: other.join_id,
            });
        }
        if self.attempt != other.attempt {
            return Err(JoinError::StaleAttempt {
                current: self.attempt,
                got: other.attempt,
            }
            .into());
        }
        if self.roster != other.roster {
            return Err(CheckedJoinError::ManifestMismatch);
        }
        Ok(())
    }

    /// Validate all outer fields, exact lengths and checksum while borrowing the
    /// payload. No roster or payload allocation occurs for untrusted envelopes.
    pub(crate) fn payload<'a>(
        &self,
        magic: &[u8; 4],
        bytes: &'a [u8],
    ) -> Result<&'a [u8], CheckedJoinError> {
        let body = join::unseal(bytes)?;
        let mut r = join::Reader { bytes: body };
        if r.take(4)? != magic {
            return Err(JoinError::BadMagic.into());
        }
        let version = r.u32()?;
        if version != VERSION {
            return Err(JoinError::UnsupportedVersion(version).into());
        }
        let id = r.u64()?;
        let attempt = r.u32()?;
        let player_count = r.u8()?;
        let joiner = r.u8()?;
        let donor = r.u8()?;
        let count = r.u32()?;
        if count == 0 || count > u32::from(player_count.saturating_sub(1)) {
            return Err(CheckedJoinError::ManifestMismatch);
        }
        let peers = r.take(count as usize)?;
        let len = r.u32()?;
        if u64::from(len) != r.bytes.len() as u64 {
            return Err(
                JoinError::Corrupt("checked payload length does not match envelope").into(),
            );
        }
        if id == 0 || attempt == 0 {
            return Err(CheckedJoinError::InvalidContext);
        }
        if id != self.join_id {
            return Err(CheckedJoinError::GenerationMismatch {
                expected: self.join_id,
                got: id,
            });
        }
        if attempt != self.attempt {
            return Err(JoinError::StaleAttempt {
                current: self.attempt,
                got: attempt,
            }
            .into());
        }
        if player_count != self.roster.player_count()
            || joiner != self.roster.joiner().0
            || donor != self.roster.donor().0
            || peers.len() != self.roster.peers().len()
            || !peers
                .iter()
                .zip(self.roster.peers())
                .all(|(&a, b)| a == b.0)
        {
            return Err(CheckedJoinError::ManifestMismatch);
        }
        let payload = r.bytes;
        self.check_inner(magic, payload)?;
        Ok(payload)
    }

    fn check_inner(&self, magic: &[u8; 4], payload: &[u8]) -> Result<(), CheckedJoinError> {
        match magic {
            b"ORRQ" => {
                let req = join::decode_request(payload)?;
                if req.joiner_id != self.join_id
                    || req.attempt != self.attempt
                    || req.backlog_peers != self.roster.peers().len() as u32
                    || req.header.player_count != self.roster.player_count()
                    || req.header.slot != self.roster.joiner()
                {
                    return Err(CheckedJoinError::InnerMismatch);
                }
            }
            b"ORRJ" => {
                let (header, ticket) = join::inspect_snapshot(payload)?;
                if ticket.attempt != self.attempt
                    || header.player_count != self.roster.player_count()
                    || header.slot != self.roster.joiner()
                {
                    return Err(CheckedJoinError::InnerMismatch);
                }
            }
            b"ORRB" => {
                // Check notice layout without allocating spans. The legacy
                // decoder remains responsible for constructing validated spans.
                let mut r = join::Reader {
                    bytes: join::unseal(payload)?,
                };
                if r.take(4)? != magic {
                    return Err(JoinError::BadMagic.into());
                }
                let version = r.u32()?;
                if version != 2 {
                    return Err(JoinError::UnsupportedVersion(version).into());
                }
                if r.u32()? != self.attempt {
                    return Err(CheckedJoinError::InnerMismatch);
                }
                let sender = PlayerSlot(r.u8()?);
                if !self.roster.peers().contains(&sender) {
                    return Err(CheckedJoinError::InnerMismatch);
                }
                let count = r.u32()?;
                if u64::from(count) * 17 != r.bytes.len() as u64 {
                    return Err(JoinError::Corrupt("span count does not match length").into());
                }
                for _ in 0..count {
                    if r.u8()? >= self.roster.player_count() {
                        return Err(CheckedJoinError::InnerMismatch);
                    }
                    let from = r.u64()?;
                    if r.u64()? <= from {
                        return Err(JoinError::Corrupt("empty span").into());
                    }
                }
            }
            _ => return Err(JoinError::BadMagic.into()),
        }
        Ok(())
    }

    // Deliberately not public: arbitrary v2 messages must not be blessed with a
    // fresh context. Production egress uses accepted request/ticket adapters.
    pub(crate) fn wrap(
        &self,
        magic: &[u8; 4],
        payload: &[u8],
    ) -> Result<Vec<u8>, CheckedJoinError> {
        self.check_inner(magic, payload)?;
        let len = u32::try_from(payload.len())
            .map_err(|_| JoinError::Corrupt("checked payload too large"))?;
        let mut out = Vec::new();
        out.extend_from_slice(magic);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.join_id.to_le_bytes());
        out.extend_from_slice(&self.attempt.to_le_bytes());
        out.extend_from_slice(&[
            self.roster.player_count(),
            self.roster.joiner().0,
            self.roster.donor().0,
        ]);
        out.extend_from_slice(&(self.roster.peers().len() as u32).to_le_bytes());
        out.extend(self.roster.peers().iter().map(|slot| slot.0));
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(payload);
        join::seal(&mut out);
        Ok(out)
    }
}

/// Created by accepting a checked request on its donor, or by importing that
/// donor's checked snapshot control metadata. Fields cannot be changed or
/// reconstructed without validating the expected context. Across processes,
/// use `import_checked_join_ticket` on the snapshot bytes received from the
/// caller-verified donor. This is not an authentication token.
#[derive(Clone, Debug)]
pub struct CheckedJoinTicket {
    context: CheckedJoinContext,
    ticket: JoinTicket,
}
impl CheckedJoinTicket {
    pub fn context(&self) -> &CheckedJoinContext {
        &self.context
    }
    /// Explicit legacy interoperability only; this copy carries no generation
    /// fence. Prefer `checked_backlog_notice` for checked peer control egress.
    pub fn ticket(&self) -> JoinTicket {
        self.ticket
    }
}

/// Validate the expected generation and full manifest before `serve_join` can
/// mutate the donor's grants/default-input ownership. `expected` comes from the
/// caller's completed membership, never from the incoming request. The caller
/// must verify that the transport sender is the expected joiner.
pub fn serve_checked_join<G: Game, S: InputSource<G>>(
    donor: &mut Session<G, S>,
    expected: &CheckedJoinContext,
    request: &[u8],
) -> Result<(Vec<u8>, CheckedJoinTicket), CheckedJoinError> {
    if donor.config().local_slot != expected.roster.donor()
        || donor.config().player_count != expected.roster.player_count()
        || donor.config().relay
    {
        return Err(CheckedJoinError::ManifestMismatch);
    }
    let payload = expected.payload(b"ORRQ", request)?;
    let snapshot = donor.serve_join(payload)?;
    let ticket = donor
        .pending_join(expected.roster.joiner())
        .ok_or(CheckedJoinError::MissingTicket)?;
    let snapshot = expected.wrap(b"ORRJ", &snapshot)?;
    Ok((
        snapshot,
        CheckedJoinTicket {
            context: expected.clone(),
            ticket,
        },
    ))
}

/// Validate an accepted ticket against the peer's independently known context,
/// install the legacy input hold, and derive its notice from that same ticket.
/// No arbitrary public byte-wrapping API exists. Call before backlog transfer;
/// the caller verifies the ticket's donor and scopes input packets to this join.
/// Reusing a ticket after handoff or membership change is not remote cleanup.
pub fn checked_backlog_notice<G: Game, S: InputSource<G>>(
    peer: &mut Session<G, S>,
    expected: &CheckedJoinContext,
    ticket: &CheckedJoinTicket,
) -> Result<Vec<u8>, CheckedJoinError> {
    expected.check(&ticket.context)?;
    if peer.config().player_count != expected.roster.player_count()
        || peer.config().relay
        || !expected.roster.peers().contains(&peer.config().local_slot)
    {
        return Err(CheckedJoinError::ManifestMismatch);
    }
    peer.hold_inputs_for_join(ticket.ticket);
    let notice = peer
        .backlog_notice(ticket.ticket.slot)
        .ok_or(CheckedJoinError::MissingTicket)?;
    ticket.context.wrap(b"ORRB", &notice)
}

/// Import a donor's accepted ticket across a process/transport boundary using
/// the existing checked snapshot wire message; there is no separate ticket wire
/// format. `donor` is the caller-verified transport identity, and `expected` must
/// come from the peer's independently completed membership/current attempt.
///
/// Checks the complete wire limit, generation, attempt, manifest, message
/// checksums and inner snapshot metadata, including build/seed/tick-rate against
/// this peer (retaining the existing zero-build-id wildcard). This only reads
/// the peer and never installs an input hold. The
/// immutable ticket can then be passed to `checked_backlog_notice`.
///
/// No frame decompression, game-state validation or frame checksum validation
/// occurs here. The joiner must still accept the snapshot normally. Checksums
/// detect corruption, not authentication; the caller must verify the donor.
pub fn import_checked_join_ticket<G: Game, S: InputSource<G>>(
    peer: &Session<G, S>,
    expected: &CheckedJoinContext,
    donor: PlayerSlot,
    snapshot: &[u8],
    snapshot_wire_limit: usize,
) -> Result<CheckedJoinTicket, CheckedJoinError> {
    if donor != expected.roster.donor() {
        return Err(CheckedJoinError::WrongDonor(donor));
    }
    if snapshot_wire_limit == 0 || snapshot.len() > snapshot_wire_limit {
        return Err(CheckedJoinError::WireLimit {
            limit: snapshot_wire_limit,
        });
    }
    if peer.config().player_count != expected.roster.player_count()
        || peer.config().relay
        || !expected.roster.peers().contains(&peer.config().local_slot)
    {
        return Err(CheckedJoinError::ManifestMismatch);
    }
    let payload = expected.payload(b"ORRJ", snapshot)?;
    let (header, ticket) = join::inspect_snapshot(payload)?;
    header.check_against(peer.config(), peer.build_hash())?;
    Ok(CheckedJoinTicket {
        context: expected.clone(),
        ticket,
    })
}
