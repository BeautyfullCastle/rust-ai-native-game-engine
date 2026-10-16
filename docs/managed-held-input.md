# Managed held input (ERP, opt-in)

The normal Arena host enables managed held input in addition to its normal
reflected input adapter. Embedders opt in with `ErpServer::enable_managed_input()`
after `set_structured_input`. No public configuration struct gains a required
field. A disabled host does not advertise this feature.

`registry.input.managed_held` and `rpc.discover.engine.input.managed_held` advertise
`{version:1, lease_ms:2000, heartbeat_ms:500}`. Clients must check this feature;
there is no raw-input fallback for managed control.

## Wire contract

All methods require the existing `sim_control` capability. Grants are bound to
the server connection, not the token's display name. IDs and sequences are
unsigned 64-bit decimal **strings**. Player slots are ordinary JSON integers.

- `sim.input_claim {player, replace_held:true}` explicitly neutralizes that
  slot's previous legacy held input and returns
  `{player, grant, generation, lease_ms:2000, accepted_head_tick}`. The replaced
  value is never restored. A live grant cannot be stolen, even by its owner;
  release it before claiming again. Claiming does not start playback or branch.
- `sim.input_value {player, grant, generation, sequence, value}` replaces held
  input through the normal complete reflected adapter. Sequence starts at `"1"`
  and must strictly increase across updates, renewals and release. Normal Arena
  FIRE therefore derives normal commands and records them normally.
- `sim.input_renew {player, grant, generation, sequence}` refreshes the grant
  without changing input. Send around every 500 ms when held input is unchanged.
- `sim.input_release {player, grant, generation, sequence}` neutralizes the
  matching slot and revokes its grant.

Update, renewal and release return
`{ok:true, player, grant, generation, sequence, accepted_head_tick}`.
`accepted_head_tick` is the numeric host tick at request acceptance, **not** an
assertion that a simulation tick or rendered frame has consumed the input.

`sim.state.managed_held` contains `{generation, slots:[{player, grant,
generation, owned_by_you}]}`. Only active grants appear. An absent slot is free;
it does not identify or describe any legacy writer. This status and discovery
are observations; successful claim is the authority to send managed input.

## Lifetime and compatibility

Valid updates and renewals extend a fixed two-second wall-clock lease. Expiry
is enforced during host service/request boundaries and before `Host::frame`
advances real-time simulation; long synchronous calls do not promise hard
real-time expiry. Timers and ownership are outside deterministic simulation
state. Their resulting inputs are recorded through the unchanged simulation
path.

Release, timeout and owning-connection disconnect neutralize only currently
matching grants. Old releases, updates or disconnects cannot clear a newer
owner. An expired/stale tagged input never falls back to legacy input. Invalid
values, sequences and claims do not refresh a lease or change held input.
Successful ERP pause, seek, stop, start and explicit Branch synchronously
invalidate managed grants. Reacquisition is explicit. Viewer rejects ownership
and input; explicit ERP Branch can make it writable before a fresh claim.

`Host::play` remains public. An embedder replacing it or driving lifecycle
controls outside ERP **must call** `server.invalidate_managed_input(&mut target)`
before the transition/replacement. Arbitrary out-of-band session changes cannot
be reliably detected from reused tick/epoch numbers.

Outside an active grant, untagged structured input and raw input retain their
previous behavior, including held-input persistence after disconnect. While a
slot is leased, both raw writes and untagged structured writes to it fail,
including untagged writes from the owner. Other slots remain independent.
After release/expiry, an untagged legacy write is again accepted: the server
cannot tell whether it was delayed or newly intended.

This coordinates **held input only**. It is not authentication or isolation
from authorized explicit commands, debug edits, timeline controls or other
simulation operations. Already accepted commands are never removed by claim,
release, expiry or invalidation. No credentials or permissions are created.
