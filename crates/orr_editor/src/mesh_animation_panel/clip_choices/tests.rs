use super::label;

#[test]
fn duplicate_and_empty_names_retain_distinct_indices() {
    assert_ne!(label(0, "walk", 1.0), label(1, "walk", 1.0));
    assert_eq!(label(3, "", 1.5), "Clip 3: (unnamed) (1.500s)");
    assert_eq!(label(4, "  ", 1.5), "Clip 4: (unnamed) (1.500s)");
    assert_eq!(label(2, "walk", 2.0), "Clip 2: walk (2.000s)");
}
