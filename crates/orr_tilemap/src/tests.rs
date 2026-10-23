use super::*;
use orr_sprite::{Atlas, Region, SpriteSource};

fn atlas() -> SpriteDocument {
    SpriteDocument::new(SpriteSource {
        format: orr_sprite::FORMAT.into(),
        version: orr_sprite::VERSION,
        atlas: Atlas {
            image: "atlas.png".into(),
            width: 64,
            height: 32,
        },
        regions: vec![
            Region {
                id: 0,
                x: 16,
                y: 8,
                width: 8,
                height: 4,
            },
            Region {
                id: 7,
                x: 0,
                y: 0,
                width: 8,
                height: 4,
            },
            // Valid but differently-sized unused region must not reject a map.
            Region {
                id: 9,
                x: 32,
                y: 0,
                width: 16,
                height: 16,
            },
        ],
        clips: vec![],
    })
    .unwrap()
}
fn source() -> Document {
    Document {
        version: VERSION,
        sprite: SpriteRef {
            package: "tiles".into(),
            document: "data/sprites.json".into(),
        },
        width: 3,
        height: 2,
        tile_size: [8, 4],
        origin: [-4.0, 10.0],
        units_per_pixel: 0.5,
        cells: vec![Some(0), None, Some(7), None, Some(0), Some(7)],
    }
}
fn bytes(doc: &Document) -> Vec<u8> {
    serde_json::to_vec(doc).unwrap()
}

#[test]
fn strict_json_and_resource_bounds() {
    let good = String::from_utf8(bytes(&source())).unwrap();
    assert_eq!(Document::from_bytes(good.as_bytes()).unwrap(), source());
    let value: serde_json::Value = serde_json::from_str(&good).unwrap();
    let mut cases = vec![
        ("top-level array", "[]".into()),
        ("positional document", r#"[1,{"package":"tiles","document":"data/sprites.json"},3,2,[8,4],[-4,10],0.5,[0,null,7,null,0,7]]"#.into()),
        ("overflowing JSON scale", good.replace("\"units_per_pixel\":0.5", "\"units_per_pixel\":1e39")),
        ("overflowing JSON origin", good.replace("[-4.0,10.0]", "[1e39,10.0]")),
        ("top-level null", "null".into()),
        ("trailing JSON", format!("{good} {{}}")),
        ("unknown", good.replacen('{', "{\"surprise\":0,", 1)),
        ("duplicate", good.replacen('{', "{\"version\":1,", 1)),
        (
            "duplicate nested",
            good.replace("\"package\":", "\"package\":\"other\",\"package\":"),
        ),
        (
            "unknown nested",
            good.replace("\"package\":", "\"extra\":0,\"package\":"),
        ),
    ];
    for (name, field, replacement) in [
        (
            "sprite array",
            "sprite",
            serde_json::json!(["tiles", "data/sprites.json"]),
        ),
        ("sprite null", "sprite", serde_json::Value::Null),
        ("cells null", "cells", serde_json::Value::Null),
        ("cells object", "cells", serde_json::json!({})),
        (
            "negative region",
            "cells",
            serde_json::json!([-1, null, 7, null, 0, 7]),
        ),
        (
            "overflow region",
            "cells",
            serde_json::json!([4294967296u64, null, 7, null, 0, 7]),
        ),
        (
            "float region",
            "cells",
            serde_json::json!([0.5, null, 7, null, 0, 7]),
        ),
        (
            "bool region",
            "cells",
            serde_json::json!([true, null, 7, null, 0, 7]),
        ),
        (
            "oversized cells",
            "cells",
            serde_json::json!(vec![None::<u32>; MAX_CELLS + 1]),
        ),
    ] {
        let mut v = value.clone();
        v[field] = replacement;
        cases.push((name, v.to_string()));
    }
    // Every field is required, including cells and nested reference members.
    for field in [
        "version",
        "sprite",
        "width",
        "height",
        "tile_size",
        "origin",
        "units_per_pixel",
        "cells",
    ] {
        let mut v = value.clone();
        v.as_object_mut().unwrap().remove(field);
        assert!(
            Document::from_bytes(v.to_string().as_bytes()).is_err(),
            "missing {field}"
        );
    }
    for field in ["package", "document"] {
        let mut v = value.clone();
        v["sprite"].as_object_mut().unwrap().remove(field);
        assert!(
            Document::from_bytes(v.to_string().as_bytes()).is_err(),
            "missing sprite.{field}"
        );
    }
    for (name, json) in cases {
        assert!(
            Document::from_bytes(json.as_bytes()).is_err(),
            "accepted {name}"
        );
    }
    let mut exact = bytes(&source());
    exact.resize(MAX_BYTES, b' ');
    assert!(Document::from_bytes(&exact).is_ok());
    exact.push(b' ');
    assert!(Document::from_bytes(&exact)
        .unwrap_err()
        .contains("byte limit"));
    let mut maximum = source();
    maximum.width = 64;
    maximum.height = 64;
    maximum.cells = vec![Some(u32::MAX); MAX_CELLS];
    assert!(bytes(&maximum).len() < MAX_BYTES);
    assert!(Document::from_bytes(&bytes(&maximum)).is_ok());
    maximum.cells.push(None);
    assert!(maximum.validate_structure().is_err());
}

#[test]
fn structural_bounds_and_finite_world_arithmetic() {
    let base = source();
    for (width, height) in [(0, 2), (3, 0), (65, 2), (3, 65), (u32::MAX, u32::MAX)] {
        let mut d = base.clone();
        d.width = width;
        d.height = height;
        assert!(d.validate_structure().is_err(), "{width}x{height}");
    }
    for cells in [vec![], vec![None; 5], vec![None; 7]] {
        let mut d = base.clone();
        d.cells = cells;
        assert!(d.validate_structure().is_err());
    }
    for size in [[0, 4], [8, 0], [2049, 4], [8, u32::MAX]] {
        let mut d = base.clone();
        d.tile_size = size;
        assert!(d.validate_structure().is_err());
    }
    for scale in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        -1.0,
        0.0,
        0.00001,
        100.1,
        f32::MAX,
    ] {
        let mut d = base.clone();
        d.units_per_pixel = scale;
        assert!(d.validate_structure().is_err());
    }
    for axis in 0..2 {
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut d = base.clone();
            d.origin[axis] = v;
            assert!(d.validate_structure().is_err());
        }
    }
    for path in [
        "", " ", "/a", "a/", "a//b", ".", "..", "a/./b", "a/../b", "C:/a", "a\\b",
    ] {
        let mut d = base.clone();
        d.sprite.document = path.into();
        assert!(d.validate_structure().is_err(), "{path}");
    }
    for package in ["", " "] {
        let mut d = base.clone();
        d.sprite.package = package.into();
        assert!(d.validate_structure().is_err());
    }
    for version in [0, 2, u32::MAX] {
        let mut d = base.clone();
        d.version = version;
        assert!(d.validate_structure().unwrap_err().contains("version"));
    }
    for scale in [MIN_UNITS_PER_PIXEL, MAX_UNITS_PER_PIXEL] {
        let mut d = base.clone();
        d.units_per_pixel = scale;
        assert!(d.validate_structure().is_ok());
    }
    // Valid dimension/tile/scale caps already bound multiplication to 13,107,200;
    // adversarial overflowing dimensions/scale are rejected before placement.
    let mut maximum = base;
    maximum.width = 64;
    maximum.height = 64;
    maximum.cells = vec![None; MAX_CELLS];
    maximum.tile_size = [2048, 2048];
    maximum.units_per_pixel = 100.0;
    maximum.origin = [-100.0, 100.0];
    assert!(maximum.validate_structure().is_ok());
    assert_eq!(
        maximum.position(CellCoord {
            column: 63,
            row: 63
        }),
        [13_004_700.0, -13_004_700.0]
    );
}

#[test]
fn region_validation_and_failure_preserving_edits() {
    let atlas = atlas();
    let source = source();
    let validated = source.validate(&atlas).unwrap();
    assert_eq!(validated.occupied_count(), 4);
    assert_eq!(
        validated.cell(CellCoord { column: 0, row: 0 }).unwrap(),
        Some(0)
    );
    assert_eq!(
        validated.cell(CellCoord { column: 1, row: 0 }).unwrap(),
        None
    );
    for coord in [
        CellCoord { column: 3, row: 0 },
        CellCoord { column: 0, row: 2 },
        CellCoord {
            column: u32::MAX,
            row: u32::MAX,
        },
    ] {
        assert!(validated.cell(coord).is_err());
        assert!(validated.with_cell(coord, None, &atlas).is_err());
    }
    let target = CellCoord { column: 1, row: 0 };
    for (id, reason) in [(99, "nonexistent"), (9, "tile_size")] {
        assert!(validated
            .with_cell(target, Some(id), &atlas)
            .unwrap_err()
            .contains(reason));
        assert_eq!(validated.document(), &source);
    }
    let painted = validated.with_cell(target, Some(0), &atlas).unwrap();
    assert_eq!(painted.occupied_count(), 5);
    assert_eq!(painted.cell(target).unwrap(), Some(0));
    assert_eq!(painted.with_cell(target, Some(0), &atlas).unwrap(), painted);
    assert_eq!(painted.with_cell(target, None, &atlas).unwrap(), validated);
    assert_eq!(
        validated.with_cell(target, None, &atlas).unwrap(),
        validated
    );
    assert_eq!(validated.document(), &source);
    // Revalidation is never skipped on a no-op with a changed atlas.
    let missing_regions = SpriteDocument::new(SpriteSource {
        format: orr_sprite::FORMAT.into(),
        version: orr_sprite::VERSION,
        atlas: atlas.atlas().clone(),
        regions: vec![],
        clips: vec![],
    })
    .unwrap();
    assert!(validated.with_cell(target, None, &missing_regions).is_err());
    assert_eq!(validated.document(), &source);
    let mut other = source.clone();
    other.tile_size = [16, 16];
    other.cells.fill(Some(9));
    assert!(other.validate(&atlas).is_ok());
    other.cells[0] = Some(0);
    assert!(other.validate(&atlas).is_err());
}

#[test]
fn asymmetric_literal_world_and_uv_oracle() {
    let atlas = atlas();
    let validated = source().validate(&atlas).unwrap();
    let placements: Vec<_> = validated.placements().collect();
    let expected = [
        (0, 0, 0, [-2.0, 9.0]),
        (2, 0, 7, [6.0, 9.0]),
        (1, 1, 0, [2.0, 7.0]),
        (2, 1, 7, [6.0, 7.0]),
    ];
    assert_eq!(placements.len(), expected.len());
    for (actual, (column, row, region, position)) in placements.iter().zip(expected) {
        assert_eq!(
            *actual,
            Placement {
                cell: CellCoord { column, row },
                region,
                position,
                size: [4.0, 2.0]
            }
        );
        // World extents: left -4, right 8, top 10, bottom 6.
        assert!(actual.position[0] - 2.0 >= -4.0 && actual.position[0] + 2.0 <= 8.0);
        assert!(actual.position[1] + 1.0 <= 10.0 && actual.position[1] - 1.0 >= 6.0);
    }
    assert_eq!(atlas.uv_rect(0), Some([0.25, 0.25, 0.375, 0.375]));
    assert_eq!(atlas.uv_rect(7), Some([0.0, 0.0, 0.125, 0.125]));
    let mut empty = source();
    empty.cells.fill(None);
    let empty = empty.validate(&atlas).unwrap();
    assert_eq!(empty.occupied_count(), 0);
    assert_eq!(empty.placements().count(), 0);
}
