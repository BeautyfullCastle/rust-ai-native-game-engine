use std::collections::BTreeMap;

use orr_proto::{
    Bundle, Channel, ClientMsg, ConnId, Endpoint, ProtoError, RejectReason, ServerEvent, ServerMsg, PROTOCOL_VERSION,
};

use crate::room::{Out, Room, RoomConfig, RoomStats, ServerNote};
use crate::validate::{AcceptAll, InputValidator};

/// How long a rejected connection is kept open so the reject message can
/// arrive before the close (microseconds).
const REJECT_GRACE_US: u64 = 2_000_000;

struct ConnInfo {
    room: Option<u64>,
}

/// The relay server core: rooms of players on top of an [`Endpoint`].
///
/// The core never reads a clock: the caller passes the current time (any
/// monotonic microsecond counter) to [`update`](Self::update), which does
/// everything that is due at that time. With a virtual clock and the
/// simulated network from `orr_proto::netsim` a whole session is
/// reproducible.
pub struct RelayServer<E: Endpoint, V: InputValidator = AcceptAll> {
    endpoint: E,
    validator: V,
    rooms: BTreeMap<u64, Room>,
    conns: BTreeMap<ConnId, ConnInfo>,
    closing: BTreeMap<ConnId, u64>,
    notes: Vec<ServerNote>,
    token_state: u64,
    bad_messages: u64,
}

impl<E: Endpoint> RelayServer<E, AcceptAll> {
    /// A plain relay (no validation). `token_seed` seeds the slot tokens
    /// handed to clients.
    pub fn new(endpoint: E, token_seed: u64) -> Self {
        Self::with_validator(endpoint, AcceptAll, token_seed)
    }
}

impl<E: Endpoint, V: InputValidator> RelayServer<E, V> {
    /// Relay + Validate: `validator` checks every input.
    pub fn with_validator(endpoint: E, validator: V, token_seed: u64) -> Self {
        Self {
            endpoint,
            validator,
            rooms: BTreeMap::new(),
            conns: BTreeMap::new(),
            closing: BTreeMap::new(),
            notes: Vec::new(),
            token_state: token_seed,
            bad_messages: 0,
        }
    }

    /// Creates a room. Clients name it by `id` in their `Hello`.
    pub fn create_room(&mut self, id: u64, cfg: RoomConfig) {
        self.rooms.insert(id, Room::new(id, cfg));
    }

    pub fn close_room(&mut self, id: u64) {
        if self.rooms.remove(&id).is_some() {
            self.notes.push(ServerNote::RoomClosed { room: id });
        }
    }

    pub fn endpoint(&self) -> &E {
        &self.endpoint
    }

    pub fn endpoint_mut(&mut self) -> &mut E {
        &mut self.endpoint
    }

    /// Notes recorded since the last call.
    pub fn drain_notes(&mut self) -> Vec<ServerNote> {
        std::mem::take(&mut self.notes)
    }

    pub fn room_stats(&self, id: u64) -> Option<RoomStats> {
        self.rooms.get(&id).map(Room::stats)
    }

    /// Last finalized tick of a room.
    pub fn finalized_tick(&self, id: u64) -> Option<u64> {
        self.rooms.get(&id).map(Room::finalized)
    }

    pub fn is_running(&self, id: u64) -> bool {
        self.rooms.get(&id).is_some_and(Room::is_running)
    }

    /// Every confirmed bundle of a room, from tick 1, when its config has
    /// `record_all`.
    pub fn recorded(&self, id: u64) -> &[Bundle] {
        self.rooms.get(&id).map_or(&[], Room::recorded)
    }

    /// Messages that failed to decode (or came from the wrong state).
    pub fn bad_messages(&self) -> u64 {
        self.bad_messages
    }

    fn next_token(&mut self) -> u64 {
        // SplitMix64; never 0.
        self.token_state = self.token_state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.token_state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) | 1
    }

    /// Does everything due at server time `now_us`: reads the endpoint,
    /// finalizes ticks whose deadline passed, sends confirmed bundles, time
    /// sync and other messages.
    pub fn update(&mut self, now_us: u64) {
        while let Some(ev) = self.endpoint.poll() {
            self.on_event(ev, now_us);
        }
        let mut closed = Vec::new();
        for (&id, room) in &mut self.rooms {
            let mut out = Out { ep: &mut self.endpoint, notes: &mut self.notes, now_us };
            room.advance(&mut out);
            if room.should_close() {
                closed.push(id);
            }
        }
        for id in closed {
            self.close_room(id);
        }
        let due: Vec<ConnId> = self.closing.iter().filter(|(_, &at)| at <= now_us).map(|(&c, _)| c).collect();
        for c in due {
            self.closing.remove(&c);
            self.endpoint.disconnect(c);
        }
    }

    fn on_event(&mut self, ev: ServerEvent, now_us: u64) {
        match ev {
            ServerEvent::Connected(conn) => {
                self.conns.insert(conn, ConnInfo { room: None });
            }
            ServerEvent::Disconnected(conn) => {
                self.closing.remove(&conn);
                let Some(info) = self.conns.remove(&conn) else { return };
                if let Some(room) = info.room.and_then(|r| self.rooms.get_mut(&r)) {
                    if let Some(slot) = room.slot_of(conn) {
                        let mut out = Out { ep: &mut self.endpoint, notes: &mut self.notes, now_us };
                        room.vacate(&mut out, slot, true);
                    }
                }
            }
            ServerEvent::Message { conn, data, .. } => self.on_message(conn, &data, now_us),
        }
    }

    fn reject(&mut self, conn: ConnId, reason: RejectReason, now_us: u64) {
        self.endpoint.send(conn, Channel::Reliable, &ServerMsg::Reject(reason).encode());
        self.notes.push(ServerNote::Rejected { conn, reason });
        self.closing.insert(conn, now_us + REJECT_GRACE_US);
    }

    fn on_message(&mut self, conn: ConnId, data: &[u8], now_us: u64) {
        let msg = match ClientMsg::decode(data) {
            Ok(m) => m,
            Err(e) => {
                self.bad_messages += 1;
                self.notes.push(ServerNote::BadMessage { conn });
                if matches!(e, ProtoError::UnsupportedVersion(_)) && self.conns.get(&conn).is_some_and(|c| c.room.is_none()) {
                    self.reject(conn, RejectReason::BadVersion { server: PROTOCOL_VERSION }, now_us);
                }
                return;
            }
        };
        let Some(info) = self.conns.get_mut(&conn) else { return };
        let room_id = info.room;
        match (msg, room_id) {
            (ClientMsg::Hello(h), None) => {
                let Some(room) = self.rooms.get(&h.room) else {
                    self.reject(conn, RejectReason::NoSuchRoom, now_us);
                    return;
                };
                let _ = room;
                let token = self.next_token();
                let room = self.rooms.get_mut(&h.room).unwrap();
                let mut out = Out { ep: &mut self.endpoint, notes: &mut self.notes, now_us };
                match room.hello(&mut out, conn, &h, token) {
                    Ok((_slot, stale)) => {
                        if let Some(info) = self.conns.get_mut(&conn) {
                            info.room = Some(h.room);
                        }
                        if let Some(old) = stale {
                            self.conns.remove(&old);
                            self.endpoint.disconnect(old);
                        }
                    }
                    Err(reason) => self.reject(conn, reason, now_us),
                }
            }
            (ClientMsg::Hello(_), Some(_)) => self.bad_messages += 1,
            (_, None) => self.bad_messages += 1,
            (msg, Some(room_id)) => {
                let Some(room) = self.rooms.get_mut(&room_id) else { return };
                let Some(slot) = room.slot_of(conn) else {
                    self.bad_messages += 1;
                    return;
                };
                let mut out = Out { ep: &mut self.endpoint, notes: &mut self.notes, now_us };
                match msg {
                    ClientMsg::Hello(_) => unreachable!("handled above"),
                    ClientMsg::Input { slot: claimed, ack_tick, entries, .. } => {
                        room.input(&mut out, &mut self.validator, slot, claimed, ack_tick, entries)
                    }
                    ClientMsg::Ping { seq, client_time_us, rtt_hint_us } => {
                        room.ping(&mut out, slot, seq, client_time_us, rtt_hint_us)
                    }
                    ClientMsg::Checksum { tick, checksum } => room.checksum(&mut out, slot, tick, checksum),
                    ClientMsg::SnapshotUpload { request_id, tick, checksum, data } => {
                        room.snapshot_uploaded(&mut out, slot, request_id, tick, checksum, data)
                    }
                    ClientMsg::SnapshotDecline { request_id } => room.snapshot_declined(&mut out, slot, request_id),
                    ClientMsg::Ready => room.ready(&mut out, slot),
                    ClientMsg::Leave => {
                        room.vacate(&mut out, slot, true);
                        self.closing.insert(conn, now_us + REJECT_GRACE_US);
                    }
                }
            }
        }
    }
}
