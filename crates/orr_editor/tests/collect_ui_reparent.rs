#![cfg(feature = "collect-ui")]
#[path = "../src/collect_ui_reparent.rs"]
mod collect_ui_reparent;

use collect_ui_reparent::{Proposal, propose};
use orr_sample::authored_ui::{Action, Binding, Document, Kind, Node, SCHEMA, Screen};
use orr_sample::collect_ui::layout_document;

fn node(id: &str, parent: Option<&str>, offset: [i16; 2], size: [u16; 2]) -> Node {
    Node {
        id: id.into(),
        parent: parent.map(str::to_owned),
        kind: Kind::Container,
        screen: Screen::Title,
        anchor: [0, 0],
        offset,
        size,
    }
}
fn fixture(anchor: [u16; 2]) -> Document {
    let mut source = node("source", Some("old"), [9, 13], [60, 40]);
    source.anchor = anchor;
    let mut leaf = node("leaf", Some("source"), [-3, 7], [37, 19]);
    leaf.anchor = [500, 250];
    leaf.kind = Kind::Button {
        text: "Play".into(),
        action: Action::Play,
    };
    let mut label = node("label", Some("source"), [4, -8], [17, 11]);
    label.kind = Kind::Label {
        text: "Score".into(),
        binding: Some(Binding::Score),
    };
    Document {
        schema: SCHEMA,
        nodes: vec![
            node("old", None, [10, 20], [200, 120]),
            node("target", None, [30, 40], [120, 80]),
            source,
            node("interleaved", None, [1, 2], [19, 23]),
            leaf,
            label,
        ],
    }
}

#[test]
fn reparent_contract_cpu_batch() {
    // Multiple anchors, unequal dimensions and interleaved descendants. Moving
    // to viewport and container both preserve every production rectangle.
    for anchor in [[0, 0], [500, 500], [1000, 1000], [250, 750], [750, 250]] {
        for target in [Some("target"), None] {
            let doc = fixture(anchor);
            let original = doc.clone();
            let Proposal::Changed(result) = propose(&doc, "source", target, [640, 360]).unwrap()
            else {
                panic!("expected changed candidate");
            };
            assert_eq!(doc, original);
            result.validate().unwrap();
            for (i, (before, after)) in doc.nodes.iter().zip(&result.nodes).enumerate() {
                if i == 2 {
                    let mut expected = before.clone();
                    expected.parent = target.map(str::to_owned);
                    expected.offset = after.offset;
                    assert_eq!(&expected, after);
                } else {
                    assert_eq!(before, after);
                }
            }
            let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 360.0));
            let before = layout_document(&doc, Screen::Title, bounds).unwrap();
            let after = layout_document(&result, Screen::Title, bounds).unwrap();
            assert!(
                before
                    .iter()
                    .zip(&after)
                    .all(|(a, b)| a.index() == b.index() && a.rect() == b.rect())
            );
            // Clipping is allowed to change and must not be sold as visual or
            // pointer-equivalence: the new ancestors supply clipping bounds.
            assert!(before.iter().zip(&after).any(|(a, b)| a.clip() != b.clip()));
            assert_eq!(
                propose(&doc, "source", Some("old"), [640, 360]).unwrap(),
                Proposal::Unchanged
            );
        }
    }
    for viewport in [[240, 240], [2048, 2048]] {
        let doc = fixture([0, 0]);
        assert_eq!(
            propose(&doc, "old", None, viewport).unwrap(),
            Proposal::Unchanged
        );
        assert!(matches!(
            propose(&doc, "source", None, viewport),
            Ok(Proposal::Changed(_))
        ));
    }
    // Nested target/source and boundary offsets exercise limits without rounding.
    let mut nested = fixture([0, 0]);
    nested.nodes[1].parent = Some("old".into());
    assert!(matches!(
        propose(&nested, "source", Some("target"), [640, 360]),
        Ok(Proposal::Changed(_))
    ));
    for bound in [-4096, 4096] {
        let mut doc = fixture([0, 0]);
        doc.nodes[0].offset = [0, 0];
        doc.nodes[1].offset = [0, 0];
        doc.nodes[2].offset = [bound, bound];
        assert!(matches!(
            propose(&doc, "source", Some("target"), [640, 360]),
            Ok(Proposal::Changed(_))
        ));
    }
    let base = fixture([0, 0]);
    type Failure = (
        &'static str,
        Document,
        &'static str,
        Option<&'static str>,
        [u32; 2],
    );
    let mut failures: Vec<Failure> = vec![
        ("missing source", base.clone(), "missing", None, [640, 360]),
        (
            "missing target",
            base.clone(),
            "source",
            Some("missing"),
            [640, 360],
        ),
        ("self", base.clone(), "source", Some("source"), [640, 360]),
        (
            "descendant",
            base.clone(),
            "source",
            Some("leaf"),
            [640, 360],
        ),
        (
            "forward",
            base.clone(),
            "source",
            Some("interleaved"),
            [640, 360],
        ),
        ("zero viewport", base.clone(), "source", None, [0, 360]),
        ("small viewport", base.clone(), "source", None, [239, 360]),
        ("large viewport", base.clone(), "source", None, [640, 2049]),
    ];
    for (name, modify) in [
        ("noncontainer", 0),
        ("cross screen", 1),
        ("fraction", 2),
        ("offset overflow", 3),
        ("cycle", 4),
        ("invalid geometry", 5),
    ] {
        let mut doc = base.clone();
        match modify {
            0 => {
                doc.nodes[1].kind = Kind::Label {
                    text: "target".into(),
                    binding: None,
                }
            }
            1 => doc.nodes[1].screen = Screen::Menu,
            2 => {
                doc.nodes[2].anchor = [333, 0];
                doc.nodes[1].size[0] = 121;
            }
            3 => {
                doc.nodes[0].offset = [4096, 0];
                doc.nodes[1].offset = [-4096, 0];
            }
            4 => doc.nodes[0].parent = Some("source".into()),
            5 => doc.nodes[2].offset = [4097, 0],
            _ => unreachable!(),
        }
        failures.push((name, doc, "source", Some("target"), [640, 360]));
    }
    // Exact arithmetic permits x offset -1227, but production f32 grouping
    // differs by one ULP. The candidate must reject rather than drift.
    let mut float_drift = base.clone();
    float_drift.nodes[0].anchor[0] = 143;
    float_drift.nodes[0].offset[0] = 0;
    float_drift.nodes[0].size[0] = 1669;
    float_drift.nodes[1].anchor[0] = 398;
    float_drift.nodes[1].offset[0] = 0;
    float_drift.nodes[1].size[0] = 3542;
    float_drift.nodes[2].anchor[0] = 600;
    float_drift.nodes[2].offset[0] = 60;
    assert!(
        propose(&float_drift, "source", Some("target"), [640, 360])
            .unwrap_err()
            .contains("production layout")
    );
    failures.push((
        "production f32 drift",
        float_drift,
        "source",
        Some("target"),
        [640, 360],
    ));
    let mut deep = base.clone();
    deep.nodes
        .insert(1, node("middle", Some("old"), [0, 0], [200, 120]));
    deep.nodes[2].parent = Some("middle".into());
    deep.validate().unwrap();
    failures.push(("subtree depth", deep, "source", Some("target"), [640, 360]));
    for (name, doc, source, target, viewport) in failures {
        let original = doc.clone();
        assert!(propose(&doc, source, target, viewport).is_err(), "{name}");
        assert_eq!(doc, original, "failure mutated source: {name}");
    }
}
