//! Independent literal fixture algebra. No production sampler/matrix/deformer is
//! used for expected values. Import is used only to load the original mesh.
#![allow(clippy::float_arithmetic)]
use orr_model::animation::{
    AnimatedModel, AnimationChannel, AnimationClip, ChannelValues, Interpolation,
};
pub type Matrix = [[f64; 4]; 4];
pub const TRANSITION: f64 = 1.0;
pub const DURATION: f64 = 5.0;
pub const POSITIONS: [[f64; 3]; 8] = [
    [-0.28, 0., 0.],
    [0.42, 0., 0.],
    [-0.22, 1., 0.],
    [0.34, 1., 0.],
    [-0.35, 2., 0.],
    [0.27, 2., 0.],
    [-0.18, 3., 0.],
    [0.46, 3., 0.],
];
pub const TRIANGLES: [[usize; 3]; 6] = [
    [0, 1, 2],
    [1, 3, 2],
    [2, 3, 4],
    [3, 5, 4],
    [4, 5, 6],
    [5, 7, 6],
];
pub const CHECKER: [[usize; 4]; 4] = [[0, 0, 1, 1], [0, 3, 1, 2], [2, 2, 3, 3], [2, 1, 3, 0]];
pub const COLORS: [[u8; 4]; 4] = [
    [245, 113, 40, 255],
    [35, 190, 213, 255],
    [133, 85, 219, 255],
    [250, 231, 166, 255],
];
#[derive(Clone, Copy, Debug)]
pub struct Local {
    pub translation: [f64; 3],
    pub rotation: [f64; 4],
    pub scale: [f64; 3],
}
const ID: Matrix = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];
const REST: Local = Local {
    translation: [0.; 3],
    rotation: [0., 0., 0., 1.],
    scale: [1.; 3],
};
fn rotation(degrees: f64) -> [f64; 4] {
    let (s, c) = (degrees.to_radians() / 2.).sin_cos();
    [0., 0., s, c]
}
pub fn fixture() -> AnimatedModel {
    let model = orr_model::animation_import::import_with_resolver(
        "original.glb",
        include_bytes!("../fixtures/animated_strip.glb"),
        |_| panic!("embedded fixture only"),
    )
    .unwrap();
    let mut source = model.source().clone();
    source.clips = (0..2)
        .map(|clip| {
            let channel = |node, times, values| AnimationChannel {
                node,
                times,
                values,
                interpolation: Interpolation::Linear,
            };
            let (t, values, angles, scale) = if clip == 0 {
                (
                    vec![0., 2.],
                    vec![[0.3, 0.2, 0.], [0.7, 0.2, 0.]],
                    [0., 60., 30.],
                    [1., 1., 1.],
                )
            } else {
                (
                    vec![0.5, 2.],
                    vec![[-0.7, 0.2, 0.], [-0.3, 0.2, 0.]],
                    [-60., 0., -30.],
                    [1.15, 0.85, 1.],
                )
            };
            let mut channels = vec![
                channel(0, t, ChannelValues::Translation(values)),
                channel(
                    2,
                    vec![0.25, 1., 1.75],
                    ChannelValues::Rotation(
                        angles
                            .map(|a| {
                                rotation(a).map(|v| (v * if clip == 1 { -1. } else { 1. }) as f32)
                            })
                            .to_vec(),
                    ),
                ),
                channel(1, vec![0.75], ChannelValues::Scale(vec![scale])),
            ];
            if clip == 1 {
                channels.reverse();
            }
            AnimationClip {
                name: format!("literal-{clip}"),
                channels,
            }
        })
        .collect();
    AnimatedModel::new(source).unwrap()
}
fn lerp<const N: usize>(a: [f64; N], b: [f64; N], t: f64) -> [f64; N] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}
fn normalize(q: [f64; 4]) -> [f64; 4] {
    let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    q.map(|v| v / n)
}
pub fn slerp(a: [f64; 4], b: [f64; 4], t: f64) -> [f64; 4] {
    let a = normalize(a);
    let mut b = normalize(b);
    let mut dot = (0..4).map(|i| a[i] * b[i]).sum::<f64>();
    if dot < 0. {
        b = b.map(|v| -v);
        dot = -dot;
    }
    if dot > 0.9995 {
        return normalize(lerp(a, b, t));
    }
    let theta = dot.clamp(-1., 1.).acos();
    let den = theta.sin();
    normalize(std::array::from_fn(|i| {
        ((1. - t) * theta).sin() / den * a[i] + (t * theta).sin() / den * b[i]
    }))
}
pub fn source_local(clip: usize, time: f64) -> Vec<Local> {
    assert!(clip < 2);
    let time = time.clamp(0., 2.);
    let mut nodes = vec![REST; 4];
    nodes[0].translation = if clip == 0 {
        lerp([0.3, 0.2, 0.], [0.7, 0.2, 0.], time / 2.)
    } else {
        lerp(
            [-0.7, 0.2, 0.],
            [-0.3, 0.2, 0.],
            ((time - 0.5) / 1.5).clamp(0., 1.),
        )
    };
    nodes[1].scale = if clip == 0 { [1.; 3] } else { [1.15, 0.85, 1.] };
    nodes[2].translation = [0., 1.5, 0.];
    nodes[3].translation = [7., 0., 0.];
    let angles = if clip == 0 {
        [0., 60., 30.]
    } else {
        [-60., 0., -30.]
    };
    let segment = usize::from(time > 1.);
    let start = [0.25, 1.][segment];
    nodes[2].rotation = slerp(
        rotation(angles[segment]),
        rotation(angles[segment + 1]),
        ((time - start) / 0.75).clamp(0., 1.),
    );
    nodes
}
pub fn sequence_local(time: f64) -> Vec<Local> {
    if time <= 2. {
        return source_local(0, time);
    }
    if time >= 3. {
        return source_local(1, time - 3.);
    }
    source_local(0, 2.)
        .into_iter()
        .zip(source_local(1, 0.))
        .map(|(a, b)| Local {
            translation: lerp(a.translation, b.translation, time - 2.),
            rotation: slerp(a.rotation, b.rotation, time - 2.),
            scale: lerp(a.scale, b.scale, time - 2.),
        })
        .collect()
}
fn matrix(n: Local) -> Matrix {
    // Rodrigues rotation from a normalized quaternion, written independently
    // as row equations then transposed into the public column-major layout.
    let [x, y, z, w] = normalize(n.rotation);
    let rows = [
        [
            1. - 2. * (y * y + z * z),
            2. * (x * y - z * w),
            2. * (x * z + y * w),
        ],
        [
            2. * (x * y + z * w),
            1. - 2. * (x * x + z * z),
            2. * (y * z - x * w),
        ],
        [
            2. * (x * z - y * w),
            2. * (y * z + x * w),
            1. - 2. * (x * x + y * y),
        ],
    ];
    let mut out = ID;
    for c in 0..3 {
        for r in 0..3 {
            out[c][r] = rows[r][c] * n.scale[c];
        }
        out[3][c] = n.translation[c];
    }
    out
}
fn multiply(a: Matrix, b: Matrix) -> Matrix {
    std::array::from_fn(|c| std::array::from_fn(|r| (0..4).map(|k| a[k][r] * b[c][k]).sum()))
}
fn globals(nodes: Vec<Local>) -> Vec<Matrix> {
    let mut m: Vec<_> = nodes.into_iter().map(matrix).collect();
    m[1] = multiply(m[0], m[1]);
    m[2] = multiply(m[1], m[2]);
    m
}
fn palette(g: &[Matrix]) -> Vec<Matrix> {
    [1, 2]
        .into_iter()
        .map(|i| {
            let mut ib = ID;
            ib[3] = [-0.3, if i == 1 { -0.2 } else { -1.7 }, 0., 1.];
            multiply(g[i], ib)
        })
        .collect()
}
fn positions(g: &[Matrix]) -> Vec<[f64; 3]> {
    let p = palette(g);
    POSITIONS
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let w = [0., 0.2, 0.7, 1.][i / 2];
            std::array::from_fn(|r| {
                (0..2)
                    .map(|j| {
                        let weight = if j == 0 { 1. - w } else { w };
                        weight * ((0..3).map(|c| p[j][c][r] * v[c]).sum::<f64>() + p[j][3][r])
                    })
                    .sum()
            })
        })
        .collect()
}
pub fn sequence_globals(time: f64) -> Vec<Matrix> {
    globals(sequence_local(time))
}
pub fn sequence_palette(time: f64) -> Vec<Matrix> {
    palette(&sequence_globals(time))
}
pub fn sequence_positions(time: f64) -> Vec<[f64; 3]> {
    positions(&sequence_globals(time))
}
pub fn source_positions(clip: usize, time: f64) -> Vec<[f64; 3]> {
    positions(&globals(source_local(clip, time)))
}

/// Explicit independent wrong-order / wrong-duration controls for consumers.
pub fn ordered_positions(first: usize, second: usize, time: f64, transition: f64) -> Vec<[f64; 3]> {
    let local = if time <= 2. {
        source_local(first, time)
    } else if time >= 2. + transition {
        source_local(second, time - 2. - transition)
    } else {
        let t = (time - 2.) / transition;
        source_local(first, 2.)
            .into_iter()
            .zip(source_local(second, 0.))
            .map(|(a, b)| Local {
                translation: lerp(a.translation, b.translation, t),
                rotation: slerp(a.rotation, b.rotation, t),
                scale: lerp(a.scale, b.scale, t),
            })
            .collect()
    };
    positions(&globals(local))
}
