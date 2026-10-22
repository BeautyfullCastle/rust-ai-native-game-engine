//! Additive, immutable action-profile admission. No callback runs during Play.
use super::*;
use orr_input::{HeldActionProfile, HeldActions};

/// Companion to an unchanged `StaticGameDescriptor`. Bits follow profile order.
pub struct StaticActionDescriptor {
    pub game_id: &'static str,
    pub profile: HeldActionProfile,
    pub held: fn(HeldActions, &TypeRegistry) -> Result<J, String>,
}
pub(super) struct FrozenActions {
    pub profile: HeldActionProfile,
    pub values: Vec<J>,
}
const REGISTRY_BYTES: usize = 4 * 1024 * 1024;
fn account(value: &J, total: &mut usize) -> Result<(), String> {
    *total = total
        .checked_add(serde_json::to_vec(value).map_err(|e| e.to_string())?.len())
        .ok_or("static action registry budget overflow")?;
    if *total > REGISTRY_BYTES {
        return Err("static action registry exceeds 4 MiB frozen JSON budget".into());
    }
    Ok(())
}
impl StaticGameRegistry {
    /// Opt specific registrations into independent held actions. All schemas,
    /// legacy values and action values share a registry-wide frozen JSON budget.
    pub fn with_action_profiles(
        descriptors: Vec<StaticGameDescriptor>,
        profiles: Vec<StaticActionDescriptor>,
    ) -> Result<Self, String> {
        if profiles.len() > 32 {
            return Err("too many static action profiles".into());
        }
        let mut seen = BTreeSet::new();
        for profile in &profiles {
            if !seen.insert(profile.game_id)
                || !descriptors.iter().any(|d| d.game_id == profile.game_id)
            {
                return Err("duplicate or unknown static action game id".into());
            }
        }
        let mut registry = Self::new(descriptors)?;
        let mut total = 0;
        for entry in &registry.entries {
            account(&entry.identity.schema, &mut total)?;
            account(entry.input.schema(), &mut total)?;
            account(
                &serde_json::json!({"game":entry.identity.game,"build":entry.identity.build_id,
                "alias":entry.descriptor.local_alias,"hint":entry.descriptor.input.hint,
                "type":entry.descriptor.input.type_name,"format":entry.input.value_format(),
                "players":entry.input.max_players()}),
                &mut total,
            )?;
            for value in &entry.held {
                account(value, &mut total)?;
            }
        }
        for descriptor in profiles {
            account(
                &serde_json::to_value(descriptor.profile.actions()).map_err(|e| e.to_string())?,
                &mut total,
            )?;
            let entry = registry
                .entries
                .iter_mut()
                .find(|e| e.descriptor.game_id == descriptor.game_id)
                .expect("companion namespace was validated");
            let info = entry
                .input_types
                .get(entry.descriptor.input.type_name)
                .ok_or("missing input type")?;
            let mut values = Vec::with_capacity(descriptor.profile.state_count());
            for keys in descriptor.profile.states() {
                let value = (descriptor.held)(keys, &entry.input_types)?;
                bounded_json(&value, 64 * 1024)?;
                account(&value, &mut total)?;
                let decoded = orr_remote::json::json_to_value(info.desc(), &value, false)?;
                let mut bytes = info.default_bytes();
                info.set(&mut bytes, "", decoded)
                    .map_err(|e| e.to_string())?;
                if keys == HeldActions::default() && bytes != info.default_bytes() {
                    return Err(
                        "static action neutral must equal the reflected typed default".into(),
                    );
                }
                values.push(value);
            }
            entry.actions = Some(FrozenActions {
                profile: descriptor.profile,
                values,
            });
        }
        Ok(registry)
    }
}
impl RegisteredGame {
    pub fn action_profile(self) -> Option<&'static HeldActionProfile> {
        self.entry().actions.as_ref().map(|a| &a.profile)
    }
    pub fn held_actions_value(self, keys: HeldActions) -> Option<J> {
        let actions = self.entry().actions.as_ref()?;
        actions
            .profile
            .index(keys)
            .map(|index| actions.values[index].clone())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use orr_input::{Action, Button, Key};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[repr(C)]
    #[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable, orr_reflect::Reflect)]
    struct MultiInput {
        x: i32,
        y: i32,
        jump: u32,
        fire: u32,
        interact: u32,
    }
    fn input_types() -> TypeRegistry {
        let mut types = TypeRegistry::new();
        types.register_component::<MultiInput>("MultiInput");
        types
    }
    pub(crate) fn profile() -> HeldActionProfile {
        HeldActionProfile::new(
            [("jump", Key::Space), ("fire", Key::F), ("interact", Key::E)]
                .into_iter()
                .map(|(name, key)| Action {
                    name: name.into(),
                    bindings: vec![Button::Keyboard { key }],
                })
                .collect(),
        )
        .unwrap()
    }
    pub(crate) fn value(k: HeldActions) -> J {
        json!({"x":k.x,"y":k.y,"jump":u32::from(k.actions&1!=0),"fire":u32::from(k.actions&2!=0),"interact":u32::from(k.actions&4!=0)})
    }
    fn legacy(k: StaticHeldKeys) -> J {
        value(HeldActions {
            x: k.x,
            y: k.y,
            actions: u8::from(k.action),
        })
    }
    fn descriptor() -> StaticGameDescriptor {
        let mut d = super::super::tests::descriptor("MultiGame", "multi-game");
        d.input.type_name = "MultiInput";
        d.input.types = input_types;
        d.input.held = legacy;
        d
    }
    fn companion() -> StaticActionDescriptor {
        StaticActionDescriptor {
            game_id: "MultiGame",
            profile: profile(),
            held: |keys, _| Ok(value(keys)),
        }
    }
    pub(crate) fn game() -> RegisteredGame {
        Box::leak(Box::new(
            StaticGameRegistry::with_action_profiles(vec![descriptor()], vec![companion()])
                .unwrap(),
        ))
        .handle("MultiGame")
        .unwrap()
    }
    #[test]
    fn frozen_multiaction_states_and_canonical_neutral() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        fn counted(k: HeldActions, _: &TypeRegistry) -> Result<J, String> {
            CALLS.fetch_add(1, Ordering::SeqCst);
            Ok(value(k))
        }
        let mut c = companion();
        c.held = counted;
        let r = Box::leak(Box::new(
            StaticGameRegistry::with_action_profiles(vec![descriptor()], vec![c]).unwrap(),
        ));
        let g = r.handle("MultiGame").unwrap();
        assert_eq!(CALLS.load(Ordering::SeqCst), 72);
        for k in profile().states() {
            assert_eq!(g.held_actions_value(k), Some(value(k)));
        }
        assert_eq!(CALLS.load(Ordering::SeqCst), 72);
        assert_eq!(g.neutral_value(), value(HeldActions::default()));
        assert!(g
            .held_actions_value(HeldActions {
                x: 2,
                y: 0,
                actions: 0
            })
            .is_none());
        assert!(g
            .held_actions_value(HeldActions {
                x: 0,
                y: 0,
                actions: 8
            })
            .is_none());
    }
    #[test]
    fn companion_namespace_and_neutral_fail_closed() {
        let mut d = descriptor();
        d.types = || panic!("callbacks must not run");
        let mut c = companion();
        c.game_id = "missing";
        assert!(StaticGameRegistry::with_action_profiles(vec![d], vec![c]).is_err());
        assert!(StaticGameRegistry::with_action_profiles(
            vec![descriptor()],
            vec![companion(), companion()]
        )
        .is_err());
        let mut c = companion();
        c.held = |_, _| {
            Ok(value(HeldActions {
                x: 1,
                y: 0,
                actions: 0,
            }))
        };
        assert!(StaticGameRegistry::with_action_profiles(vec![descriptor()], vec![c]).is_err());
    }
    #[test]
    fn callback_errors_abort_admission_without_publishing() {
        let mut c = companion();
        c.held = |_, _| Err("typed mapping rejected sample".into());
        let error = StaticGameRegistry::with_action_profiles(vec![descriptor()], vec![c])
            .err()
            .unwrap();
        assert_eq!(error, "typed mapping rejected sample");
    }
    #[test]
    fn input_type_factory_is_frozen_once() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        fn once() -> TypeRegistry {
            assert_eq!(CALLS.fetch_add(1, Ordering::SeqCst), 0);
            input_types()
        }
        let mut d = descriptor();
        d.input.types = once;
        assert!(StaticGameRegistry::with_action_profiles(vec![d], vec![companion()]).is_ok());
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable, orr_reflect::Reflect)]
    struct LargeInput {
        cells: [[[u32; 16]; 16]; 16],
    }
    fn large_types() -> TypeRegistry {
        let mut types = TypeRegistry::new();
        types.register_component::<LargeInput>("LargeInput");
        types
    }
    fn large_value(_: HeldActions) -> J {
        let cells = [[[0u32; 16]; 16]; 16];
        serde_json::json!({"cells": cells})
    }
    #[test]
    fn admission_enforces_aggregate_across_individually_bounded_tables() {
        let names = [
            ("Large0", "large0"),
            ("Large1", "large1"),
            ("Large2", "large2"),
            ("Large3", "large3"),
            ("Large4", "large4"),
            ("Large5", "large5"),
            ("Large6", "large6"),
            ("Large7", "large7"),
        ];
        assert!(
            serde_json::to_vec(&large_value(HeldActions::default()))
                .unwrap()
                .len()
                < 64 * 1024
        );
        let mut descriptors = Vec::new();
        let mut profiles = Vec::new();
        for (id, alias) in names {
            let mut d = super::super::tests::descriptor(id, alias);
            d.input.type_name = "LargeInput";
            d.input.types = large_types;
            d.input.held = |_| large_value(HeldActions::default());
            descriptors.push(d);
            profiles.push(StaticActionDescriptor {
                game_id: id,
                profile: profile(),
                held: |keys, _| Ok(large_value(keys)),
            });
        }
        let error = StaticGameRegistry::with_action_profiles(descriptors, profiles)
            .err()
            .unwrap();
        assert!(error.contains("4 MiB"), "{error}");
    }
    #[test]
    fn aggregate_budget_is_not_only_per_value() {
        let mut total = REGISTRY_BYTES - 2;
        assert!(account(&json!("over"), &mut total).is_err());
    }
}
