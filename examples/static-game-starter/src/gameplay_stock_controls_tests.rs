// CPU-only reproduction of the original registered control gap. The manifest
// and scene bytes are stock; victory reads below use ordinary reflected ERP.
fn stock_fixture(alias: &str, scene: &str) -> Fixture {
    use orr_editor::registered_project::PreparedRegisteredProject;
    let (dir, path) = saved_project(alias, scene);
    let prepared = PreparedRegisteredProject::open(&path, registry()).unwrap();
    assert_eq!(prepared.scene_text(), scene);
    let contract = prepared.game().validated().clone();
    let mut config = ServerConfig::new(Auth::DevNoAuth);
    config.listen = false;
    let (work_status, _owner) = orr_remote::work_status::WorkStatusService::pair();
    let host = (contract.host_factory())(orr_editor::static_adapter::StaticHostRequest {
        scene_text: prepared.scene_text().into(),
        scene_path: prepared.scene_path().into(),
        config,
        work_status,
        neutral_value: contract.neutral_value().clone(),
        expected_input: contract.expected_input().clone(),
    })
    .unwrap();
    assert!(
        host.url().is_none(),
        "CPU diagnostic must not open a socket"
    );
    let mut observer = ErpClient::with_transport(Box::new(
        host.connector()
            .connect("stock-observer", Caps::parse("read").unwrap())
            .unwrap(),
    ));
    observer.call_timeout = Duration::from_secs(1);
    let mut remote = RemoteConfig::new("in-process");
    remote.view_delivery = ViewDeliveryMode::RequireFenced;
    let router = GameplayRouter::connect_local(
        contract,
        &host,
        "stock-registered-controls",
        Caps::ALL,
        remote,
    )
    .unwrap();
    Fixture {
        _dir: dir,
        _host: host,
        observer,
        router,
    }
}

// A Play ACK can precede the stream's resumed frame. Observe both public
// sources before one claim; a stale paused frame with tick > 0 is not live.
fn stock_router_claim_ready(router: &GameplayRouter) -> bool {
    router.input_phase() == orr_editor::editor::input::Phase::Off
        && router.error().is_none()
        && router.state()["mode"] == "play"
        && router.state()["playing"] == true
        && router.state()["session_mode"] == "record"
        && router.state()["head_tick"] == router.state()["last_tick"]
        && router.frame_evidence().is_some_and(|evidence| {
            let snapshot = evidence.snapshot();
            let tick = snapshot.tick();
            tick > 0
                && tick == evidence.stamp().tick
                && snapshot.predicted().tick() == tick
                && snapshot.timeline().is_some_and(|timeline| {
                    timeline.mode == orr_bridge::PlayMode::Record
                        && timeline.playing
                        && timeline.tick == tick
                        && timeline.tick == timeline.last_tick
                })
        })
}

fn reflected_stock_state(observer: &mut ErpClient, actor: &str, singleton: &str) -> J {
    let query = observer
        .call("world.query", json!({"components":[actor],"values":true}))
        .unwrap();
    assert_eq!(query["total"], 1);
    let guid = &query["entities"][0]["guid"];
    let actor_state = observer
        .call("world.get", json!({"entity":guid,"component":actor}))
        .unwrap();
    let singleton_state = observer
        .call("world.singleton.get", json!({"name":singleton}))
        .unwrap();
    json!({"actor":actor_state["value"],"singleton":singleton_state["value"]})
}

fn assert_stock_files_unchanged(dir: &std::path::Path, alias: &str, scene: &str) {
    let manifest = match alias {
        "delivery" => include_bytes!("../scenes/delivery.project.json").as_slice(),
        "switchboard" => include_bytes!("../scenes/switchboard.project.json").as_slice(),
        _ => unreachable!(),
    };
    assert_eq!(std::fs::read(dir.join("project.json")).unwrap(), manifest);
    assert_eq!(
        std::fs::read_to_string(dir.join(format!("{alias}.scene.yaml"))).unwrap(),
        scene
    );
}

#[test]
fn stock_registered_act_only_diagnostic_preserves_unreachable_objectives() {
    for (alias, scene, actor, singleton) in [
        ("delivery", crate::delivery::SCENE, "Courier", "Delivery"),
        ("switchboard", crate::switchboard::SCENE, "Cursor", "Board"),
    ] {
        let Fixture {
            _dir,
            _host,
            mut observer,
            mut router,
        } = stock_fixture(alias, scene);
        let contract = registry().get(alias).unwrap();
        let catalog: Vec<_> = contract.intent_catalog().collect();
        assert!(catalog.iter().any(|entry| entry.id == "act"));
        let schemas = observer.call("registry.schema", J::Null).unwrap();
        assert_eq!(schemas["schema"], contract.identity().schema);
        let raw_input = observer.call("registry.input", J::Null).unwrap();
        contract.expected_input().validate(&raw_input).unwrap();
        println!("stock {alias} catalog={catalog:?} raw_input={raw_input}");
        let original = observer.call("scene.save", J::Null).unwrap();
        let history = observer.call("history.list", J::Null).unwrap();
        let initial = reflected_stock_state(&mut observer, actor, singleton);
        println!("stock {alias} initial={initial}");
        assert_eq!(initial["actor"]["position"], json!([0, 0]));
        if alias == "delivery" {
            assert_eq!(initial["singleton"]["goal"], json!([4, 0]));
        } else {
            assert_eq!(initial["singleton"]["target"], 5);
        }
        router.play().unwrap();
        wait(&mut router, stock_router_claim_ready);
        router.claim(0).unwrap();
        wait(&mut router, GameplayRouter::can_submit);
        for count in 1..=4 {
            let id = router.submit(act()).unwrap();
            wait(&mut router, |router| {
                matches!(router.status(id), Some(CommandStatus::Completed { .. }))
            });
            let Some(CommandStatus::Completed { consumed, neutral }) = router.status(id) else {
                panic!("real consumption and later neutral evidence required");
            };
            assert_eq!(consumed.scope, neutral.scope);
            assert!(neutral.tick > consumed.tick);
            let state = reflected_stock_state(&mut observer, actor, singleton);
            println!("stock {alias} act={count} command={} consumed_tick={} neutral_tick={} state={state}",
                id.0, consumed.tick, neutral.tick);
            assert_eq!(state["actor"]["position"], json!([0, 0]));
            if alias == "delivery" {
                assert_eq!(state["singleton"]["won"], 0);
                assert_eq!(state["singleton"]["action_held"], 0);
                assert_eq!(state["singleton"]["goal"], json!([4, 0]));
            } else {
                assert_eq!(state["actor"]["selected"], 0);
                assert_eq!(state["actor"]["held"], 0);
                assert_eq!(state["singleton"]["bits"], count % 2);
                assert_eq!(state["singleton"]["moves"], count);
                assert_eq!(state["singleton"]["solved"], 0);
                assert_eq!(state["singleton"]["target"], 5);
            }
        }
        router.stop().unwrap();
        wait(&mut router, |router| router.state()["mode"] == "edit");
        assert_eq!(observer.call("scene.save", J::Null).unwrap(), original);
        assert_eq!(observer.call("history.list", J::Null).unwrap(), history);
        assert_stock_files_unchanged(_dir.path(), alias, scene);
        println!("stock {alias} document/history/manifest/scene unchanged");
    }
}

#[test]
fn stock_scenes_win_through_separate_raw_managed_input_positive_control() {
    for (alias, scene, actor, singleton) in [
        ("delivery", crate::delivery::SCENE, "Courier", "Delivery"),
        ("switchboard", crate::switchboard::SCENE, "Cursor", "Board"),
    ] {
        let Fixture {
            _dir,
            _host,
            mut observer,
            router,
        } = stock_fixture(alias, scene);
        drop(router);
        let original = observer.call("scene.save", J::Null).unwrap();
        let history = observer.call("history.list", J::Null).unwrap();
        // This is deliberately a different public API path. It is evidence that
        // the unchanged game/scene works, not proof of registered playability.
        let mut raw = ErpClient::with_transport(Box::new(
            _host
                .connector()
                .connect("raw-positive-control", Caps::ALL)
                .unwrap(),
        ));
        raw.call_timeout = Duration::from_secs(1);
        raw.call("sim.start", J::Null).unwrap();
        let grant = raw
            .call("sim.input_claim", json!({"player":0,"replace_held":true}))
            .unwrap();
        let trace = if alias == "delivery" {
            vec![
                (4, json!({"dx":1,"dy":0,"deliver":0})),
                (1, json!({"dx":0,"dy":0,"deliver":1})),
                (1, json!({"dx":0,"dy":0,"deliver":0})),
            ]
        } else {
            vec![
                (1, json!({"selection":0,"toggle":1})),
                (1, json!({"selection":0,"toggle":0})),
                (2, json!({"selection":1,"toggle":0})),
                (1, json!({"selection":0,"toggle":1})),
                (1, json!({"selection":0,"toggle":0})),
            ]
        };
        for (index, (ticks, value)) in trace.into_iter().enumerate() {
            let receipt = raw
                .call(
                    "sim.input_value",
                    json!({
                        "player":0,"grant":grant["grant"],"generation":grant["generation"],
                        "sequence":(index+1).to_string(),"value":value,
                    }),
                )
                .unwrap();
            let tick = raw.call("sim.step", json!({"n":ticks})).unwrap();
            let state = reflected_stock_state(&mut observer, actor, singleton);
            println!("raw-positive {alias} input={value} ticks={ticks} receipt={receipt} head_tick={} state={state}", tick["head_tick"]);
        }
        let won = reflected_stock_state(&mut observer, actor, singleton);
        if alias == "delivery" {
            assert_eq!(won["actor"]["position"], json!([4, 0]));
            assert_eq!(won["singleton"]["won"], 1);
            assert_eq!(won["singleton"]["action_held"], 0);
        } else {
            assert_eq!(won["actor"]["selected"], 2);
            assert_eq!(won["actor"]["held"], 0);
            assert_eq!(won["singleton"]["bits"], 5);
            assert_eq!(won["singleton"]["solved"], 1);
        }
        raw.call("sim.stop", json!({"include_replay":false}))
            .unwrap();
        assert_eq!(observer.call("scene.save", J::Null).unwrap(), original);
        assert_eq!(observer.call("history.list", J::Null).unwrap(), history);
        assert_stock_files_unchanged(_dir.path(), alias, scene);
        println!("raw-positive {alias} document/history/manifest/scene unchanged");
    }
}

fn complete_stock_command(
    router: &mut GameplayRouter,
    observer: &mut ErpClient,
    alias: &str,
    actor: &str,
    singleton: &str,
    request: IntentRequest,
) -> J {
    let intent = request.id().to_owned();
    let id = router.submit(request).unwrap();
    wait(router, |router| {
        matches!(router.status(id), Some(CommandStatus::Completed { .. }))
    });
    let Some(CommandStatus::Completed { consumed, neutral }) = router.status(id) else {
        panic!("semantic command requires consumption and later neutral evidence");
    };
    assert_eq!(consumed.scope, neutral.scope);
    assert!(neutral.tick > consumed.tick);
    let state = reflected_stock_state(observer, actor, singleton);
    println!("semantic {alias} intent={intent} command={} consumed_tick={} neutral_tick={} state={state}",
        id.0, consumed.tick, neutral.tick);
    if alias == "delivery" {
        assert_eq!(state["singleton"]["action_held"], 0);
    } else {
        assert_eq!(state["actor"]["held"], 0);
    }
    state
}

fn stock_registered_navigation_acceptance(alias: &str, scene: &str, actor: &str, singleton: &str) {
    let Fixture {
        _dir,
        _host,
        mut observer,
        mut router,
    } = stock_fixture(alias, scene);
    let original = observer.call("scene.save", J::Null).unwrap();
    let history = observer.call("history.list", J::Null).unwrap();
    let initial = reflected_stock_state(&mut observer, actor, singleton);
    let contract = registry().get(alias).unwrap();
    let catalog: Vec<_> = contract.intent_catalog().collect();
    let navigation = if alias == "delivery" {
        "move"
    } else {
        "select"
    };
    println!("semantic {alias} initial={initial} catalog={catalog:?}");
    assert_stock_files_unchanged(_dir.path(), alias, scene);
    assert_eq!(observer.call("scene.save", J::Null).unwrap(), original);
    assert_eq!(observer.call("history.list", J::Null).unwrap(), history);
    assert!(catalog.iter().any(|entry| entry.id == "act"));
    let capability = catalog.iter().find(|entry| entry.id == navigation);
    assert!(
        capability.is_some(),
        "stock registered {alias} must advertise {navigation} to reach its unchanged objective"
    );
    let schema = capability.unwrap().payload_schema;
    let fields: &[&str] = if alias == "delivery" {
        &["dx", "dy"]
    } else {
        &["delta"]
    };
    assert_eq!(schema["required"], json!(fields));
    assert_eq!(schema["additionalProperties"], false);
    for field in fields {
        assert_eq!(schema["properties"][field]["type"], "integer");
        assert_eq!(schema["properties"][field]["minimum"], -1);
        assert_eq!(schema["properties"][field]["maximum"], 1);
    }
    assert_eq!(initial["actor"]["position"], json!([0, 0]));
    if alias == "delivery" {
        assert_eq!(initial["singleton"]["goal"], json!([4, 0]));
    } else {
        assert_eq!(initial["singleton"]["target"], 5);
    }
    router.play().unwrap();
    wait(&mut router, stock_router_claim_ready);
    router.claim(0).unwrap();
    wait(&mut router, GameplayRouter::can_submit);
    let invalid = if alias == "delivery" {
        vec![
            json!({"dx":0,"dy":0}),
            json!({"dx":2,"dy":0}),
            json!({"dx":-2,"dy":0}),
            json!({"dx":1}),
            json!({"dx":0.5,"dy":0}),
            json!({"dx":1,"dy":0,"extra":1}),
        ]
    } else {
        vec![
            json!({"delta":0}),
            json!({"delta":2}),
            json!({"delta":-2}),
            json!({}),
            json!({"delta":0.5}),
            json!({"delta":1,"extra":1}),
        ]
    };
    for payload in invalid {
        assert!(
            router
                .submit(IntentRequest::new(navigation, payload.clone()).unwrap())
                .is_err(),
            "invalid {navigation} payload accepted: {payload}"
        );
    }
    assert!(router.can_submit());
    assert_eq!(
        reflected_stock_state(&mut observer, actor, singleton),
        initial
    );
    if alias == "delivery" {
        for x in 1..=4 {
            let state = complete_stock_command(
                &mut router,
                &mut observer,
                alias,
                actor,
                singleton,
                IntentRequest::new("move", json!({"dx":1,"dy":0})).unwrap(),
            );
            assert_eq!(state["actor"]["position"], json!([x, 0]));
            assert_eq!(state["singleton"]["won"], 0);
            assert_eq!(state["singleton"]["goal"], json!([4, 0]));
        }
    } else {
        let toggled =
            complete_stock_command(&mut router, &mut observer, alias, actor, singleton, act());
        assert_eq!(toggled["singleton"]["bits"], 1);
        assert_eq!(toggled["singleton"]["moves"], 1);
        assert_eq!(toggled["singleton"]["solved"], 0);
        for selected in 1..=2 {
            let state = complete_stock_command(
                &mut router,
                &mut observer,
                alias,
                actor,
                singleton,
                IntentRequest::new("select", json!({"delta":1})).unwrap(),
            );
            assert_eq!(state["actor"]["selected"], selected);
            assert_eq!(state["actor"]["position"], json!([selected * 2, 0]));
            assert_eq!(state["singleton"]["bits"], 1);
            assert_eq!(state["singleton"]["moves"], 1);
            assert_eq!(state["singleton"]["solved"], 0);
            assert_eq!(state["singleton"]["target"], 5);
        }
    }
    let victory =
        complete_stock_command(&mut router, &mut observer, alias, actor, singleton, act());
    if alias == "delivery" {
        assert_eq!(victory["actor"]["position"], json!([4, 0]));
        assert_eq!(victory["singleton"]["won"], 1);
    } else {
        assert_eq!(victory["actor"]["selected"], 2);
        assert_eq!(victory["singleton"]["bits"], 5);
        assert_eq!(victory["singleton"]["moves"], 2);
        assert_eq!(victory["singleton"]["solved"], 1);
    }
    let restart =
        complete_stock_command(&mut router, &mut observer, alias, actor, singleton, act());
    if alias == "delivery" {
        assert_eq!(restart["actor"]["position"], json!([0, 0]));
        assert_eq!(restart["singleton"]["won"], 0);
    } else {
        // Existing restart resets the puzzle but retains its cursor selection.
        assert_eq!(restart["actor"]["selected"], 2);
        assert_eq!(restart["singleton"]["bits"], 0);
        assert_eq!(restart["singleton"]["moves"], 0);
        assert_eq!(restart["singleton"]["solved"], 0);
    }
    router.stop().unwrap();
    wait(&mut router, |router| router.state()["mode"] == "edit");
    assert_eq!(observer.call("scene.save", J::Null).unwrap(), original);
    assert_eq!(observer.call("history.list", J::Null).unwrap(), history);
    assert_stock_files_unchanged(_dir.path(), alias, scene);
    println!(
        "semantic {alias} victory/restart complete; document/history/manifest/scene unchanged"
    );
}

#[test]
fn stock_registered_navigation_delivery_reaches_goal_and_restarts() {
    stock_registered_navigation_acceptance(
        "delivery",
        crate::delivery::SCENE,
        "Courier",
        "Delivery",
    );
}

#[test]
fn stock_registered_navigation_switchboard_solves_target_and_restarts() {
    stock_registered_navigation_acceptance(
        "switchboard",
        crate::switchboard::SCENE,
        "Cursor",
        "Board",
    );
}

fn stock_navigation(alias: &str) -> IntentRequest {
    if alias == "delivery" {
        IntentRequest::new("move", json!({"dx":1,"dy":0})).unwrap()
    } else {
        IntentRequest::new("select", json!({"delta":1})).unwrap()
    }
}

fn assert_stock_navigation_state(state: &J, alias: &str, steps: u32, held: u32) {
    if alias == "delivery" {
        assert_eq!(state["actor"]["position"], json!([steps, 0]));
        assert_eq!(state["singleton"]["won"], 0);
        assert_eq!(state["singleton"]["action_held"], held);
    } else {
        assert_eq!(state["actor"]["selected"], steps);
        assert_eq!(state["actor"]["position"], json!([steps * 2, 0]));
        assert_eq!(state["actor"]["held"], held);
        assert_eq!(state["singleton"]["bits"], 0);
        assert_eq!(state["singleton"]["moves"], 0);
        assert_eq!(state["singleton"]["solved"], 0);
    }
}

fn command_admissions(observer: &mut ErpClient, id: orr_editor::editor::input::CommandId) -> usize {
    let activity = observer
        .call("activity.list", json!({"include_reads":false,"limit":256}))
        .unwrap();
    let token = format!("command={}", id.0);
    activity["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| {
            entry["method"] == "sim.input_value"
                && entry["ok"] == true
                && entry["summary"]
                    .as_str()
                    .is_some_and(|summary| summary.split_whitespace().any(|word| word == token))
        })
        .count()
}

// The observer never pumps the command owner here. The host continues ticking
// while the command ACK and frame evidence deliberately remain unobserved.
fn wait_stock_host_ticks(observer: &mut ErpClient, ticks: u64) -> u64 {
    let start = observer.call("sim.state", J::Null).unwrap()["head_tick"]
        .as_u64()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let head = observer.call("sim.state", J::Null).unwrap()["head_tick"]
            .as_u64()
            .unwrap();
        if head >= start + ticks {
            return head;
        }
        assert!(
            Instant::now() < deadline,
            "host failed to advance below the 2000ms lease"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn stock_navigation_delayed_observation_moves_once_and_requires_newer_neutral() {
    for (alias, scene, actor, singleton) in [
        ("delivery", crate::delivery::SCENE, "Courier", "Delivery"),
        ("switchboard", crate::switchboard::SCENE, "Cursor", "Board"),
    ] {
        let Fixture {
            _dir,
            _host,
            mut observer,
            mut router,
        } = stock_fixture(alias, scene);
        let original = observer.call("scene.save", J::Null).unwrap();
        let history = observer.call("history.list", J::Null).unwrap();
        router.play().unwrap();
        wait(&mut router, stock_router_claim_ready);
        router.claim(0).unwrap();
        wait(&mut router, GameplayRouter::can_submit);
        let id = router.submit(stock_navigation(alias)).unwrap();
        router.poll(Instant::now()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while command_admissions(&mut observer, id) != 1 {
            assert!(Instant::now() < deadline, "navigation was not admitted");
            std::thread::sleep(Duration::from_millis(1));
        }
        let delayed_head = wait_stock_host_ticks(&mut observer, 8);
        assert!(
            matches!(router.status(id), Some(CommandStatus::Pending)),
            "an uncollected ACK must not prove consumption"
        );
        assert_stock_navigation_state(
            &reflected_stock_state(&mut observer, actor, singleton),
            alias,
            1,
            1,
        );
        wait(&mut router, |r| {
            matches!(r.status(id), Some(CommandStatus::Consumed { .. }))
        });
        let Some(CommandStatus::Consumed { stamp }) = router.status(id) else {
            unreachable!()
        };
        let consumed = *stamp;
        wait(&mut router, |r| {
            matches!(r.status(id), Some(CommandStatus::Completed { .. }))
        });
        let Some(CommandStatus::Completed {
            consumed: completed,
            neutral,
        }) = router.status(id)
        else {
            unreachable!()
        };
        assert_eq!(*completed, consumed);
        assert_eq!(consumed.scope, neutral.scope);
        assert!(neutral.tick > consumed.tick);
        assert_eq!(command_admissions(&mut observer, id), 1);
        assert_stock_navigation_state(
            &reflected_stock_state(&mut observer, actor, singleton),
            alias,
            1,
            0,
        );
        println!(
            "delayed {alias} host_head={delayed_head} consumed_tick={} neutral_tick={}",
            consumed.tick, neutral.tick
        );
        let next = complete_stock_command(
            &mut router,
            &mut observer,
            alias,
            actor,
            singleton,
            stock_navigation(alias),
        );
        assert_stock_navigation_state(&next, alias, 2, 0);
        router.stop().unwrap();
        wait(&mut router, |r| r.state()["mode"] == "edit");
        assert_eq!(observer.call("scene.save", J::Null).unwrap(), original);
        assert_eq!(observer.call("history.list", J::Null).unwrap(), history);
        assert_stock_files_unchanged(_dir.path(), alias, scene);
    }
}

#[test]
fn stock_navigation_lifecycle_cancels_before_and_after_admission_without_replay() {
    use orr_editor::editor::input::Phase;
    for (alias, scene, actor, singleton) in [
        ("delivery", crate::delivery::SCENE, "Courier", "Delivery"),
        ("switchboard", crate::switchboard::SCENE, "Cursor", "Board"),
    ] {
        for boundary in ["release", "focus_lost", "pause", "stop"] {
            for admitted in [false, true] {
                let Fixture {
                    _dir,
                    _host,
                    mut observer,
                    mut router,
                } = stock_fixture(alias, scene);
                let original = observer.call("scene.save", J::Null).unwrap();
                let history = observer.call("history.list", J::Null).unwrap();
                router.play().unwrap();
                wait(&mut router, stock_router_claim_ready);
                router.claim(0).unwrap();
                wait(&mut router, GameplayRouter::can_submit);
                let id = router.submit(stock_navigation(alias)).unwrap();
                if admitted {
                    router.poll(Instant::now()).unwrap();
                    let deadline = Instant::now() + Duration::from_secs(1);
                    while command_admissions(&mut observer, id) != 1 {
                        assert!(Instant::now() < deadline, "navigation was not admitted");
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    wait_stock_host_ticks(&mut observer, 2);
                    assert_stock_navigation_state(
                        &reflected_stock_state(&mut observer, actor, singleton),
                        alias,
                        1,
                        1,
                    );
                }
                match boundary {
                    "release" => router.release(),
                    "focus_lost" => router.focus_lost(),
                    "pause" => router.pause().unwrap(),
                    "stop" => router.stop().unwrap(),
                    _ => unreachable!(),
                }
                let cancelled = router.status(id).unwrap().clone();
                if admitted {
                    assert!(matches!(cancelled, CommandStatus::Uncertain { .. }));
                } else {
                    assert!(matches!(cancelled, CommandStatus::Cancelled { .. }));
                }
                wait(&mut router, |r| {
                    r.input_phase() == Phase::Off
                        && match boundary {
                            "pause" => r.state()["mode"] == "play" && r.state()["playing"] == false,
                            "stop" => r.state()["mode"] == "edit",
                            _ => true,
                        }
                });
                assert!(!router.can_submit());
                assert_eq!(router.retry(id), Some(&cancelled));
                assert_eq!(command_admissions(&mut observer, id), usize::from(admitted));
                if boundary == "pause" || boundary == "stop" {
                    router.play().unwrap();
                }
                wait(&mut router, stock_router_claim_ready);
                router.claim(0).unwrap_or_else(|error| {
                    panic!("stock reclaim {alias} boundary={boundary} admitted={admitted}: {error}; state={}", router.state())
                });
                wait(&mut router, GameplayRouter::can_submit);
                wait_stock_host_ticks(&mut observer, 3);
                router.poll(Instant::now()).unwrap();
                let previous_steps = u32::from(admitted && boundary != "stop");
                assert_stock_navigation_state(
                    &reflected_stock_state(&mut observer, actor, singleton),
                    alias,
                    previous_steps,
                    0,
                );
                assert_eq!(router.retry(id), Some(&cancelled));
                assert_eq!(command_admissions(&mut observer, id), usize::from(admitted));
                let next = complete_stock_command(
                    &mut router,
                    &mut observer,
                    alias,
                    actor,
                    singleton,
                    stock_navigation(alias),
                );
                assert_stock_navigation_state(&next, alias, previous_steps + 1, 0);
                println!("lifecycle {alias} boundary={boundary} admitted={admitted} old_status={cancelled:?} reclaimed_state={next}");
                router.stop().unwrap();
                wait(&mut router, |r| r.state()["mode"] == "edit");
                assert_eq!(observer.call("scene.save", J::Null).unwrap(), original);
                assert_eq!(observer.call("history.list", J::Null).unwrap(), history);
                assert_stock_files_unchanged(_dir.path(), alias, scene);
            }
        }
    }
}
