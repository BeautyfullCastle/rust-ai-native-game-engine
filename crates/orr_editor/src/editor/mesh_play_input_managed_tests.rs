use super::*;

fn claim_reply() -> J {
    json!({"player":0,"grant":"9","generation":"3","lease_ms":2000,"accepted_head_tick":5})
}

fn value_reply(params: &J, accepted_head_tick: u64) -> J {
    json!({
        "ok":true,
        "player":params["player"],
        "grant":params["grant"],
        "generation":params["generation"],
        "sequence":params["sequence"],
        "accepted_head_tick":accepted_head_tick
    })
}

#[test]
fn mesh_goal_queued_during_value_flight_waits_for_consumption_and_latest_wins() {
    let now = Instant::now();
    let mut input = Input::new(true);
    input.mesh_navigation = true;
    input.claim(0);
    let (_, _, claim_intent, claim_op) = input.next(now).unwrap();
    input.answer(claim_intent, claim_op, Ok(claim_reply()), now);

    let (_, first_params, first_intent, first_op) = input.next(now).unwrap();
    assert_eq!(first_op, Op::Value);
    assert!(input.mesh_goal([65_536, 0, 0]));
    input.answer(
        first_intent,
        first_op,
        Ok(value_reply(&first_params, 6)),
        now,
    );

    let (_, goal_params, goal_intent, goal_op) = input.next(now).unwrap();
    assert_eq!(goal_op, Op::Value);
    assert_eq!(
        goal_params["value"],
        json!({
            "buttons":orr_remote::mesh_navigation::MESH_SET_GOAL,
            "origin":[65_536,0,0],
            "dir":[0,0,0]
        })
    );
    assert!(input.mesh_goal([131_072, 0, 0]));
    input.answer(goal_intent, goal_op, Ok(value_reply(&goal_params, 7)), now);

    let (_, next_goal_params, next_goal_intent, next_goal_op) = input.next(now).unwrap();
    assert_eq!(next_goal_op, Op::Value);
    assert_eq!(next_goal_params["value"]["origin"], json!([131_072, 0, 0]));
    assert_eq!(
        next_goal_params["value"]["buttons"],
        orr_remote::mesh_navigation::MESH_SET_GOAL
    );
    input.answer(
        next_goal_intent,
        next_goal_op,
        Ok(value_reply(&next_goal_params, 8)),
        now,
    );
    assert!(
        input.next(now).is_none(),
        "the newest goal stays held through its acceptance acknowledgement"
    );
    input.mesh_observe_goal(9, [131_072, 0, 0]);

    let (_, neutral_params, neutral_intent, neutral_op) = input.next(now).unwrap();
    assert_eq!(neutral_op, Op::Value);
    assert_eq!(neutral_params["value"]["buttons"], 0);
    input.answer(
        neutral_intent,
        neutral_op,
        Ok(value_reply(&neutral_params, 8)),
        now,
    );
    assert!(input.next(now).is_none());
}

#[test]
fn invalid_claim_ack_discards_goal_and_does_not_emit_value() {
    let now = Instant::now();
    let mut input = Input::new(true);
    input.mesh_navigation = true;
    input.claim(0);
    assert!(input.mesh_goal([0, 0, 0]));
    let (_, _, intent, op) = input.next(now).unwrap();
    input.answer(
        intent,
        op,
        Ok(json!({"player":1,"grant":"9","generation":"3","lease_ms":2000,"accepted_head_tick":5})),
        now,
    );
    assert_eq!(input.phase, Phase::Off);
    assert!(input.next(now).is_none());
}
