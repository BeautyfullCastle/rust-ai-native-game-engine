#![allow(dead_code)]
use orr_fp::FP;
use orr_terrain::Terrain;
use orr_terrain_demo::Course;

pub fn terrain(width: u32, depth: u32, origin: [i32; 2], spacing: FP, holes: Vec<bool>) -> Terrain {
    Terrain::new(
        "test/course".into(),
        width,
        depth,
        origin.map(FP::from_int),
        spacing,
        vec![FP::ZERO; (width * depth) as usize],
        holes,
    )
    .unwrap()
}
pub fn flat() -> Terrain {
    terrain(17, 17, [-8, -8], FP::from_int(1), vec![false; 256])
}
pub fn json(spawn: [&str; 2], beacons: [[&str; 2]; 4], finish: [&str; 2]) -> Vec<u8> {
    serde_json::to_vec(
        &serde_json::json!({"version":1,"title":"Test trail","terrain":"hill_course.orrt",
        "spawn":spawn,"beacons":beacons,"finish":finish,"radius":"0.25"}),
    )
    .unwrap()
}
pub fn course(t: &Terrain, spawn: [&str; 2], beacons: [[&str; 2]; 4], finish: [&str; 2]) -> Course {
    Course::from_bytes(&json(spawn, beacons, finish), &t.cook()).unwrap()
}
pub fn simple(spawn: [&str; 2]) -> Course {
    course(
        &flat(),
        spawn,
        [["3", "3"], ["4", "4"], ["5", "5"], ["6", "6"]],
        ["7", "7"],
    )
}

/// A fixed authored keyboard script, not route search. Every segment avoids the
/// built-in hole. Cardinal lengths are exact multiples of the 1/16 tick step.
pub fn builtin_script() -> [(orr_terrain_demo::Input, u32); 10] {
    use orr_terrain_demo::Input;
    [
        (Input { x: 1, z: 0 }, 16),
        (Input { x: 0, z: -1 }, 48), // (-5,3), beacon 1
        (Input { x: 1, z: 0 }, 16),
        (Input { x: 0, z: -1 }, 80), // (-4,-2), beacon 2
        (Input { x: 0, z: -1 }, 48),
        (Input { x: 1, z: 0 }, 80), // (1,-5), beacon 3
        (Input { x: 1, z: 0 }, 64),
        (Input { x: 0, z: 1 }, 48), // (5,-2), beacon 4
        (Input { x: -1, z: 0 }, 16),
        (Input { x: 0, z: -1 }, 32), // summit (4,-4)
    ]
}
