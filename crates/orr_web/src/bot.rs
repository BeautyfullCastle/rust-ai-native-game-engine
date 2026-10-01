//! The scripted player: a pure function of `(slot, tick)`, the same family as
//! the bots of the native clients.

use orr_fp::FP;
use orr_testgame::{ArenaInput, SpawnBulletCmd, FIRE};

fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// A player-like arena input: holds a direction for a few ticks and fires now and then.
pub fn arena_bot_input(slot: u8, tick: u64) -> (ArenaInput, Vec<SpawnBulletCmd>) {
    let s = u64::from(slot);
    let h = mix((tick / 7 + s * 3) ^ (s << 40));
    let (ax, ay) = ((h % 3) as i32 - 1, ((h >> 8) % 3) as i32 - 1);
    let fire = tick % 11 == s % 11 && (h >> 20) % 3 != 0;
    let input = ArenaInput::new(FP::from_int(ax), FP::from_int(ay), fire);
    let cmds = if input.buttons & FIRE != 0 { vec![SpawnBulletCmd { owner: u32::from(slot) }] } else { Vec::new() };
    (input, cmds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bot_is_a_pure_function() {
        for slot in 0..4u8 {
            for tick in 0..200u64 {
                assert_eq!(arena_bot_input(slot, tick).0, arena_bot_input(slot, tick).0);
            }
        }
        let fires = (0..600u64).filter(|&t| arena_bot_input(1, t).0.buttons & FIRE != 0).count();
        assert!(fires > 10, "the bot fires now and then ({fires})");
    }
}
