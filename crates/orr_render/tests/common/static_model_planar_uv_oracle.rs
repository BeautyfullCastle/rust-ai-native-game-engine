//! Test-owned imported-style fixture and scalar oracle; never calls authoring code.
#![allow(dead_code, clippy::float_arithmetic)]
use orr_model::{Image, StaticModel, Wrap};
use serde_json::{json, Value};
pub const SOURCE_ID: &str = "scene/planar-source.glb";
pub const SOURCE_PATH: &str = "inputs/source.model.json";
pub const ORIGINAL: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");
pub const SIZE: u32 = 256;
pub const EPSILON: u8 = 3;
pub const POSITIONS: [[f64; 2]; 8] = [
    [-1.0, -1.0],
    [1.0, -1.0],
    [1.0, 0.4],
    [0.4, 1.0],
    [-1.0, 1.0],
    [1.1, -0.75],
    [1.8, -0.75],
    [1.4, 0.4],
];
pub const TRIANGLES: [[usize; 3]; 4] = [[0, 1, 2], [0, 2, 3], [0, 3, 4], [5, 6, 7]];
pub const TEXELS: [[u8; 4]; 16] = [
    [239, 31, 71, 255],
    [17, 211, 89, 255],
    [53, 97, 229, 255],
    [179, 137, 43, 255],
    [37, 163, 197, 255],
    [223, 61, 173, 255],
    [113, 193, 29, 255],
    [67, 47, 149, 255],
    [251, 181, 101, 255],
    [83, 229, 157, 255],
    [193, 109, 241, 255],
    [131, 73, 19, 255],
    [29, 113, 59, 255],
    [157, 41, 107, 255],
    [101, 151, 211, 255],
    [211, 199, 67, 255],
];
pub fn recipe() -> Value {
    json!({"format":"orr_static_planar_uv","version":1,"source":SOURCE_PATH,
        "material":0,"plane":"xy","origin":[0.125,-0.25,0.375],"units_per_repeat":[0.75,0.625]})
}
pub fn fixture() -> StaticModel {
    let imported =
        orr_model::import::import_with_resolver(SOURCE_ID, ORIGINAL, |_| panic!("embedded source"))
            .unwrap();
    let mut source = imported.source().clone();
    assert_eq!(source.primitives.len(), 2);
    for p in &mut source.primitives {
        p.transform = [
            [1.0, 0.25, 0.125, 0.0],
            [-0.25, 1.0, 0.25, 0.0],
            [0.5, 0.0, 1.0, 0.0],
            [-0.25, 0.125, 0.5, 1.0],
        ];
        for v in &mut p.vertices {
            let [x, y, _] = v.position;
            v.position[2] = 0.5 * x + 0.25 * y;
            v.normal = [-0.4364358, -0.2182179, 0.8728716];
            v.uv = if p.material == 0 {
                [0.0, 0.0]
            } else {
                [(x + 1.0) * 0.5, (1.0 - y) * 0.5]
            };
        }
    }
    source.images[0] = Image {
        width: 4,
        height: 4,
        rgba8: TEXELS.concat(),
    };
    for m in &mut source.materials {
        m.base_color = [1.0; 4];
        m.image = 0;
        m.linear_filter = false;
        m.wrap_s = Wrap::Repeat;
        m.wrap_t = Wrap::Repeat;
    }
    StaticModel::new(source).unwrap()
}
#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Original,
    Projected,
    WrongAxis,
    WrongRepeat,
    LocalSpace,
}
// Explicit f64 transform equations, independent of fixture matrices and produced UVs.
pub fn world([x, y]: [f64; 2]) -> [f64; 3] {
    [
        1.25 * x - 0.125 * y - 0.25,
        0.25 * x + y + 0.125,
        0.625 * x + 0.5 * y + 0.5,
    ]
}
pub fn projected(local: [f64; 2], mode: Mode) -> [f64; 2] {
    let p = world(local);
    match mode {
        Mode::Original => [0.0, 0.0],
        Mode::Projected => [(p[0] - 0.125) / 0.75, (p[1] + 0.25) / 0.625],
        Mode::WrongAxis => [(p[0] - 0.125) / 0.75, (p[2] - 0.375) / 0.625],
        Mode::WrongRepeat => [(p[0] - 0.125) / 1.5, (p[1] + 0.25) / 1.25],
        Mode::LocalSpace => [(local[0] - 0.125) / 0.75, (local[1] + 0.25) / 0.625],
    }
}
fn barycentric(p: [f64; 2], ids: [usize; 3]) -> [f64; 3] {
    let [a, b, c] = ids.map(|i| POSITIONS[i]);
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    let u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    [u, v, 1.0 - u - v]
}
pub enum Expected {
    Interior {
        color: [u8; 4],
        target: bool,
        texel: usize,
    },
    Background,
    Boundary,
}
// Camera is fixed: eye (0,0,5), target origin, ortho half-height 3.
// Inverse of the projected 2x2 affine matrix, not renderer camera utilities.
pub fn expected(x: u32, y: u32, mode: Mode) -> Expected {
    let wx = (f64::from(x) + 0.5 - 128.0) * 6.0 / 256.0 + 0.25;
    let wy = (128.0 - f64::from(y) - 0.5) * 6.0 / 256.0 - 0.125;
    let local = [
        (wx + 0.125 * wy) / 1.28125,
        (-0.25 * wx + 1.25 * wy) / 1.28125,
    ];
    let mut near = false;
    for (i, ids) in TRIANGLES.into_iter().enumerate() {
        let w = barycentric(local, ids);
        near |= w.iter().all(|&v| v >= -0.04);
        // Fixed geometric margin, never derived from actual output colors.
        if !w.iter().all(|&v| v > 0.04) {
            continue;
        }
        let uv = if i == 3 {
            [(local[0] + 1.0) * 0.5, (1.0 - local[1]) * 0.5]
        } else {
            projected(local, mode)
        };
        let scaled = uv.map(|v| v * 4.0);
        // Original all-zero coordinates deliberately sit on repeat boundary,
        // whose selected nearest texel is exactly (0,0); do not mask it out.
        if !(i != 3 && matches!(mode, Mode::Original))
            && scaled.iter().any(|v| (v - v.round()).abs() < 0.002)
        {
            return Expected::Boundary;
        }
        let texel = (scaled[1].floor() as i64).rem_euclid(4) as usize * 4
            + (scaled[0].floor() as i64).rem_euclid(4) as usize;
        return Expected::Interior {
            color: TEXELS[texel],
            target: i != 3,
            texel,
        };
    }
    if near {
        Expected::Boundary
    } else {
        Expected::Background
    }
}
pub fn matches(a: &[u8], b: &[u8]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= EPSILON)
}
pub fn assert_pixels(pixels: &[u8], mode: Mode) -> Value {
    assert_eq!(pixels.len(), (SIZE * SIZE * 4) as usize);
    let (mut target, mut marker, mut clear) = (0, 0, 0);
    let mut texels = [0usize; 16];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let offset = ((y * SIZE + x) * 4) as usize;
            let actual = &pixels[offset..offset + 4];
            match expected(x, y, mode) {
                Expected::Interior {
                    color,
                    target: is_target,
                    texel,
                } => {
                    assert!(
                        matches(actual, &color),
                        "independent {mode:?} pixel {x},{y}: {actual:?} != {color:?}"
                    );
                    if is_target {
                        target += 1;
                        texels[texel] += 1;
                    } else {
                        marker += 1;
                    }
                }
                Expected::Background => {
                    assert!(
                        matches(actual, &[0, 0, 0, 255]),
                        "background {x},{y}: {actual:?}"
                    );
                    clear += 1;
                }
                Expected::Boundary => (),
            }
        }
    }
    assert!(target > 5000, "vacuous selected coverage: {target}");
    assert!(marker > 500, "vacuous marker coverage: {marker}");
    assert!(clear > 50000, "vacuous background coverage: {clear}");
    if matches!(mode, Mode::Projected) {
        assert!(
            texels.iter().all(|&v| v > 150),
            "vacuous texture cells: {texels:?}"
        );
    }
    json!({"target":target,"non_target":marker,"background":clear,"target_texels":texels})
}
