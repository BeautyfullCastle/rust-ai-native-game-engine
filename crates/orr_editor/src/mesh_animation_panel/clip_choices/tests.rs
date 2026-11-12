use super::label;

#[test]
fn duplicate_and_empty_names_retain_distinct_indices() {
    assert_ne!(label(0, "walk", 1.0), label(1, "walk", 1.0));
    assert_eq!(label(3, "", 1.5), "Clip 3: (unnamed) (1.500s)");
    assert_eq!(label(4, "  ", 1.5), "Clip 4: (unnamed) (1.500s)");
    assert_eq!(label(2, "walk", 2.0), "Clip 2: walk (2.000s)");
}

#[test]
fn labeled_selectors_keep_independent_indices_and_report_only_changes() {
    use egui_kittest::{kittest::Queryable, Harness};
    let clips = vec![(0, "idle".into(), 1.0), (1, "walk".into(), 2.0)];
    let mut harness = Harness::builder().build_ui_state(
        move |ui, state: &mut ([String; 3], [usize; 3])| {
            for (index, name) in [
                "Moving animation clip",
                "Idle animation clip",
                "Animation preview clip choice",
            ]
            .into_iter()
            .enumerate()
            {
                if super::show_labeled(ui, name, &clips, &mut state.0[index]) {
                    state.1[index] += 1;
                }
            }
        },
        (["0".into(), "0".into(), "invalid".into()], [0; 3]),
    );
    harness.run_steps(3);
    assert_eq!(harness.state().0[2], "invalid");
    for (index, name) in [
        "Moving animation clip",
        "Idle animation clip",
        "Animation preview clip choice",
    ]
    .into_iter()
    .enumerate()
    {
        harness.get_by_label(name).click();
        harness.run_steps(3);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Clip 1: walk (2.000s)")
            .click();
        harness.run_steps(3);
        assert_eq!(harness.state().0[index], "1");
        assert_eq!(harness.state().1[index], 1);
        for untouched in index + 1..3 {
            assert_eq!(
                harness.state().0[untouched],
                if untouched == 2 { "invalid" } else { "0" }
            );
        }
        harness.get_by_label(name).click();
        harness.run_steps(3);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Clip 1: walk (2.000s)")
            .click();
        harness.run_steps(3);
        assert_eq!(
            harness.state().1[index],
            1,
            "reselecting same clip must be a no-op"
        );
    }
    assert_eq!(harness.state().1, [1; 3]);
}

#[test]
fn empty_clip_list_preserves_invalid_selection_without_reporting_change() {
    use egui_kittest::Harness;
    let mut harness = Harness::builder().build_ui_state(
        |ui, state: &mut String| {
            assert!(!super::show_labeled(
                ui,
                "Moving animation clip",
                &[],
                state
            ))
        },
        "99".to_owned(),
    );
    harness.run_steps(3);
    assert_eq!(harness.state(), "99");
}

#[test]
fn sparse_indices_and_invalid_text_are_not_normalized_on_idle_frames() {
    use egui_kittest::Harness;
    for original in [" 7 ", "99", "invalid", "", "03"] {
        let mut harness = Harness::builder().build_ui_state(
            |ui, selected: &mut String| {
                assert!(!super::show_labeled(
                    ui,
                    "Moving animation clip",
                    &[(3, "same".into(), 1.0), (7, "same".into(), 2.0)],
                    selected
                ));
            },
            original.to_owned(),
        );
        harness.run_steps(3);
        assert_eq!(harness.state(), original);
    }
}
