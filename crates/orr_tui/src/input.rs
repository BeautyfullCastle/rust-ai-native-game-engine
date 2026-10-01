//! Building the player's input bytes from the schema's INPUT LAYOUT (field offsets and types),
//! never from a Rust struct. The viewer has four abstract controls; each is bound to the field
//! of the layout that plays that role, found by name (`axis_x`, `axis_y`, `spin`, a `shoot`
//! flag), falling back to the first `fixed` fields in order for the two axes.

use crate::schema::{InputField, ViewSchema};

/// What the person holds: stick `-1..=1` on both axes, spin `-1..=1`, the action button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    pub x: i8,
    pub y: i8,
    pub spin: i8,
    pub fire: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    X,
    Y,
    Spin,
    Fire,
}

fn role_by_name(name: &str) -> Option<Role> {
    match name {
        "axis_x" | "move_x" | "x" | "stick_x" => Some(Role::X),
        "axis_y" | "move_y" | "y" | "stick_y" => Some(Role::Y),
        "spin" | "rotate" | "turn" => Some(Role::Spin),
        "shoot" | "fire" | "action" | "primary" | "jump" => Some(Role::Fire),
        _ => None,
    }
}

fn is_fixed(f: &InputField) -> bool {
    f.ty == "fixed" || f.ty == "fixed32"
}

/// Writes `value` little-endian into `size` bytes at `offset` (two's complement for negatives).
fn put(buf: &mut [u8], offset: usize, size: usize, value: i64) {
    let bytes = value.to_le_bytes();
    // Sign extension beyond 8 bytes is never needed (inputs have no 128-bit fields).
    for (i, byte) in bytes.iter().enumerate().take(size.min(8)) {
        if let Some(b) = buf.get_mut(offset + i) {
            *b = *byte;
        }
    }
}

/// The bytes of one input (exactly `schema.input_size` long, zeros where no control applies).
pub fn encode(schema: &ViewSchema, c: &Controls) -> Vec<u8> {
    let mut buf = vec![0u8; schema.input_size];
    let value = |role: Role| match role {
        Role::X => i64::from(c.x),
        Role::Y => i64::from(c.y),
        Role::Spin => i64::from(c.spin),
        Role::Fire => i64::from(c.fire),
    };
    // Named fields first.
    let mut taken = [false; 2];
    for f in &schema.input_fields {
        let by_name = role_by_name(&f.name);
        match (f.ty.as_str(), by_name) {
            ("fixed" | "fixed32", Some(r @ (Role::X | Role::Y))) => {
                taken[usize::from(r == Role::Y)] = true;
                put(&mut buf, f.offset, f.size, value(r) * 65536);
            }
            ("flags", _) => {
                // The button bit: one named like a fire role, else the first bit.
                let mask = f.bits.iter().find(|(n, _)| role_by_name(n) == Some(Role::Fire)).or_else(|| f.bits.first()).map_or(0, |(_, m)| *m);
                if c.fire {
                    put(&mut buf, f.offset, f.size, mask as i64);
                }
            }
            (_, Some(Role::Fire)) => put(&mut buf, f.offset, f.size, value(Role::Fire)),
            (_, Some(Role::Spin)) => put(&mut buf, f.offset, f.size, value(Role::Spin)),
            _ => {}
        }
    }
    // Axes without a known name: the first unnamed fixed fields, in layout order.
    let mut spare = schema.input_fields.iter().filter(|f| is_fixed(f) && role_by_name(&f.name).is_none());
    for (axis, role) in [Role::X, Role::Y].into_iter().enumerate() {
        if !taken[axis] {
            if let Some(f) = spare.next() {
                put(&mut buf, f.offset, f.size, value(role) * 65536);
            }
        }
    }
    buf
}

/// A scripted player, a pure function of `(player, tick)`: the same scenario as the C client of
/// `orr_ffi` (`tests/c/view_client.c`), so the headless result can be compared with it.
pub fn scenario(player: u32, tick: u32) -> Controls {
    let (p, t) = (player as i32, tick as i32);
    Controls { x: ((t / 20 + p) % 3 - 1) as i8, y: ((t / 30 + 2 * p) % 3 - 1) as i8, spin: ((t / 25 + p) % 3 - 1) as i8, fire: p == 0 && t % 40 < 5 }
}
