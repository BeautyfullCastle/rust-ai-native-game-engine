use orr_sprite::{
    grid::{generate, GridClip, GridSpec},
    Atlas, PlaybackMode, Region, SpriteDocument, MAX_ATLAS_DIMENSION, MAX_CLIP_ID_BYTES,
    MAX_IMAGE_NAME_BYTES, MAX_REGIONS,
};

fn inputs() -> (Atlas, GridSpec, GridClip) {
    (
        Atlas {
            image: "sprites/hero.png".into(),
            width: 37,
            height: 23,
        },
        GridSpec {
            columns: 3,
            rows: 2,
            cell_width: 7,
            cell_height: 5,
            offset_x: 4,
            offset_y: 3,
            spacing_x: 2,
            spacing_y: 4,
            first_region_id: 91,
        },
        GridClip {
            name: "walk".into(),
            first_cell: 1,
            frame_count: 4,
            duration_ms: 13,
            mode: PlaybackMode::Loop,
        },
    )
}

#[test]
fn asymmetric_top_left_row_major_coordinates_and_ids_are_exact() {
    let (a, g, c) = inputs();
    let d = generate(a, g, c).unwrap();
    let expected = [
        (91, 4, 3),
        (92, 13, 3),
        (93, 22, 3),
        (94, 4, 12),
        (95, 13, 12),
        (96, 22, 12),
    ]
    .map(|(id, x, y)| Region {
        id,
        x,
        y,
        width: 7,
        height: 5,
    });
    assert_eq!(d.regions(), expected);
    assert_eq!(d.atlas().width, 37); // Unused border is intentional.
    assert_eq!(
        d.clip("walk")
            .unwrap()
            .frames()
            .iter()
            .map(|f| f.region)
            .collect::<Vec<_>>(),
        [92, 93, 94, 95]
    );
}

#[test]
fn loop_once_sampling_and_json_round_trip_are_exact() {
    let (a, g, mut c) = inputs();
    for mode in [PlaybackMode::Loop, PlaybackMode::Once] {
        c.mode = mode;
        let d = generate(a.clone(), g, c.clone()).unwrap();
        let clip = d.clip("walk").unwrap();
        assert_eq!(clip.duration_ms(), 52);
        for (time, id) in [(0, 92), (12, 92), (13, 93), (26, 94), (39, 95), (51, 95)] {
            assert_eq!(clip.sample(time).region, id);
            assert!(!clip.sample(time).finished);
        }
        assert_eq!(
            clip.sample(52).region,
            if mode == PlaybackMode::Loop { 92 } else { 95 }
        );
        assert_eq!(clip.sample(52).finished, mode == PlaybackMode::Once);
        assert_eq!(clip.sample(u64::MAX).finished, mode == PlaybackMode::Once);
        if mode == PlaybackMode::Loop {
            assert_eq!(clip.sample(u64::MAX), clip.sample(u64::MAX % 52));
        }
        let json = d.to_json().unwrap();
        assert_eq!(
            SpriteDocument::from_json(&json).unwrap().to_json().unwrap(),
            json
        );
        assert_eq!(
            generate(a.clone(), g, c.clone())
                .unwrap()
                .to_json()
                .unwrap(),
            json
        );
    }
}

#[test]
fn atlas_and_collection_limits_are_inclusive() {
    let (mut a, mut g, mut c) = inputs();
    a.width = MAX_ATLAS_DIMENSION;
    a.height = MAX_ATLAS_DIMENSION;
    g.columns = 64;
    g.rows = 64;
    g.cell_width = 32;
    g.cell_height = 32;
    g.offset_x = 0;
    g.offset_y = 0;
    g.spacing_x = 0;
    g.spacing_y = 0;
    c.first_cell = 0;
    c.frame_count = MAX_REGIONS as u32;
    let d = generate(a.clone(), g, c.clone()).unwrap();
    assert_eq!(d.regions().len(), 4096);
    assert_eq!(d.clip("walk").unwrap().frames().len(), 4096);
    assert_eq!(
        d.regions().last().unwrap(),
        &Region {
            id: 91 + 4095,
            x: 2016,
            y: 2016,
            width: 32,
            height: 32
        }
    );
    for axis in [0, 1] {
        let mut bad = a.clone();
        if axis == 0 {
            bad.width += 1
        } else {
            bad.height += 1
        }
        assert!(generate(bad, g, c.clone()).is_err());
    }
    // 4097 cells fit the atlas, isolating the region-count bound from extents.
    let over_grid = GridSpec {
        columns: 17,
        rows: 241,
        cell_width: 1,
        cell_height: 1,
        offset_x: 0,
        offset_y: 0,
        spacing_x: 0,
        spacing_y: 0,
        first_region_id: 0,
    };
    let over_atlas = Atlas {
        image: "sheet.png".into(),
        width: 17,
        height: 241,
    };
    let one_frame = GridClip {
        first_cell: 0,
        frame_count: 1,
        ..c.clone()
    };
    let error = generate(over_atlas, over_grid, one_frame).unwrap_err();
    assert!(error.to_string().contains("region count"));
    g.rows = 65;
    assert!(generate(a.clone(), g, c.clone()).is_err());
    g.rows = 64;
    c.frame_count = 4097;
    assert!(generate(a, g, c).is_err());
}

#[test]
fn exact_last_edge_is_allowed_but_one_pixel_outside_rejects() {
    let (mut a, g, c) = inputs();
    a.width = 29;
    a.height = 17;
    assert!(generate(a.clone(), g, c.clone()).is_ok());
    a.width -= 1;
    assert!(generate(a.clone(), g, c.clone()).is_err());
    a.width += 1;
    a.height -= 1;
    assert!(generate(a, g, c).is_err());
}

#[test]
fn zero_dimensions_and_counts_reject() {
    let (a, g, c) = inputs();
    for field in 0..6 {
        let mut a = a.clone();
        let mut g = g;
        match field {
            0 => a.width = 0,
            1 => a.height = 0,
            2 => g.columns = 0,
            3 => g.rows = 0,
            4 => g.cell_width = 0,
            _ => g.cell_height = 0,
        }
        assert!(generate(a, g, c.clone()).is_err(), "field {field}");
    }
}

#[test]
fn count_extent_and_id_overflow_reject_before_generation() {
    let (a, g, c) = inputs();
    for field in 0..9 {
        let mut bad = g;
        match field {
            0 => bad.columns = u32::MAX,
            1 => bad.rows = u32::MAX,
            2 => bad.cell_width = u32::MAX,
            3 => bad.cell_height = u32::MAX,
            4 => bad.offset_x = u32::MAX,
            5 => bad.offset_y = u32::MAX,
            6 => bad.spacing_x = u32::MAX,
            7 => bad.spacing_y = u32::MAX,
            _ => bad.first_region_id = u32::MAX,
        }
        assert!(
            generate(a.clone(), bad, c.clone()).is_err(),
            "field {field}"
        );
    }
    let mut exact = g;
    exact.first_region_id = u32::MAX - 5;
    assert_eq!(
        generate(a.clone(), exact, c.clone()).unwrap().regions()[5].id,
        u32::MAX
    );
    exact.first_region_id += 1;
    assert!(generate(a, exact, c).is_err());
}

#[test]
fn clip_empty_outside_overflow_and_duration_overflow_reject() {
    let (a, g, c) = inputs();
    for (first, count, duration) in [
        (0, 0, 1),
        (6, 1, 1),
        (5, 2, 1),
        (u32::MAX, 1, 1),
        (1, u32::MAX, 1),
        (0, 1, 0),
        (0, 2, u64::MAX),
    ] {
        let bad = GridClip {
            first_cell: first,
            frame_count: count,
            duration_ms: duration,
            ..c.clone()
        };
        assert!(
            generate(a.clone(), g, bad).is_err(),
            "{first}/{count}/{duration}"
        );
    }
    let exact = GridClip {
        first_cell: 5,
        frame_count: 1,
        duration_ms: u64::MAX,
        ..c
    };
    let d = generate(a, g, exact).unwrap();
    assert_eq!(d.clips()[0].duration_ms(), u64::MAX);
    assert_eq!(d.clips()[0].sample(u64::MAX).region, 96);
}

#[test]
fn singleton_grid_ignores_unused_spacing_and_accepts_max_id() {
    let (mut a, mut g, mut c) = inputs();
    a.width = 1;
    a.height = 1;
    g.columns = 1;
    g.rows = 1;
    g.cell_width = 1;
    g.cell_height = 1;
    g.offset_x = 0;
    g.offset_y = 0;
    g.spacing_x = u32::MAX;
    g.spacing_y = u32::MAX;
    g.first_region_id = u32::MAX;
    c.first_cell = 0;
    c.frame_count = 1;
    let d = generate(a, g, c).unwrap();
    assert_eq!(
        d.regions(),
        [Region {
            id: u32::MAX,
            x: 0,
            y: 0,
            width: 1,
            height: 1
        }]
    );
}

#[test]
fn image_and_clip_names_obey_utf8_byte_bounds_and_are_inert() {
    let (mut a, g, mut c) = inputs();
    a.image = "é".repeat(MAX_IMAGE_NAME_BYTES / 2);
    c.name = "é".repeat(MAX_CLIP_ID_BYTES / 2);
    assert!(generate(a.clone(), g, c.clone()).is_ok());
    a.image.push('x');
    assert!(generate(a.clone(), g, c.clone()).is_err());
    a.image.pop();
    c.name.push('x');
    assert!(generate(a.clone(), g, c.clone()).is_err());
    c.name = "walk".into();
    for name in ["", " \t\n"] {
        a.image = name.into();
        assert!(generate(a.clone(), g, c.clone()).is_err());
    }
    a.image = "../not-present/atlas.png".into();
    for name in ["", " \t\n"] {
        c.name = name.into();
        assert!(generate(a.clone(), g, c.clone()).is_err());
    }
    c.name = "walk".into();
    assert_eq!(generate(a.clone(), g, c).unwrap().atlas(), &a);
}
