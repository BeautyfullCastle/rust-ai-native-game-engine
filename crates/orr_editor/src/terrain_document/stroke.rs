//! Bounded baseline-once drag strokes. Live candidates are rebuilt from the
//! complete starting admission, so release publishes one history transaction
//! and cancellation restores saved/dirty/history/query/model state exactly.
use super::{brush::TerrainBrush, check_render_admission, AdmittedTerrain, TerrainSession};
use orr_fp::FP;
use orr_terrain::{Edit, Terrain, MAX_EDITS};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_STROKE_CENTERS: usize = 256;
pub const MAX_STROKE_VERTICES: usize = MAX_EDITS;
pub const MAX_STROKE_FOOTPRINT_VISITS: usize = 65_536;

pub(super) struct Stroke {
    baseline: AdmittedTerrain,
    brush: TerrainBrush,
    centers: BTreeSet<[u32; 2]>,
    heights: BTreeMap<(u32, u32), FP>,
    footprint_visits: usize,
    last: [u32; 2],
}

/// Grid-cell-center traversal with simultaneous diagonal steps on exact ties.
/// Collinear subdivisions through integer centers retain the same ordered path.
/// The caller first validates endpoints against the bounded terrain grid.
fn raster_segment(from: [u32; 2], to: [u32; 2]) -> Vec<[u32; 2]> {
    let [mut x, mut z] = from.map(i64::from);
    let [tx, tz] = to.map(i64::from);
    let dx = (tx - x).abs();
    let dz = (tz - z).abs();
    let sx = (tx - x).signum();
    let sz = (tz - z).signum();
    let (mut ix, mut iz) = (0_i64, 0_i64);
    let mut centers = vec![from];
    while ix < dx || iz < dz {
        let decision = (1 + 2 * ix) * dz - (1 + 2 * iz) * dx;
        if decision <= 0 && ix < dx {
            x += sx;
            ix += 1;
        }
        if decision >= 0 && iz < dz {
            z += sz;
            iz += 1;
        }
        centers.push([x as u32, z as u32]);
    }
    centers
}

pub(crate) fn center_is_surface(terrain: &Terrain, [x, z]: [u32; 2]) -> bool {
    if x >= terrain.width() || z >= terrain.depth() {
        return false;
    }
    // Interior seams belong to +X/+Z; inclusive outer edges use the last cell.
    let cx = x.min(terrain.width() - 2);
    let cz = z.min(terrain.depth() - 2);
    !terrain.holes()[(cz * (terrain.width() - 1) + cx) as usize]
}

impl TerrainSession {
    pub fn stroke_active(&self) -> bool {
        self.stroke.is_some()
    }
    pub fn active_stroke_brush(&self) -> Option<TerrainBrush> {
        self.stroke.as_ref().map(|s| TerrainBrush {
            center: s.last,
            ..s.brush
        })
    }
    /// Pointer picking uses the initial surface, never a raised live preview.
    pub fn sculpt_pick_terrain(&self) -> Option<&Terrain> {
        self.stroke
            .as_ref()
            .map(|s| s.baseline.document.terrain())
            .or_else(|| self.terrain())
    }
    pub(super) fn require_idle(&self) -> Result<(), String> {
        if self.stroke_active() {
            Err("Finish or cancel the held terrain stroke first".into())
        } else {
            Ok(())
        }
    }
    pub fn begin_stroke(&mut self, brush: TerrainBrush) -> Result<(), String> {
        self.require_editable()?;
        let baseline = self
            .admitted
            .as_ref()
            .ok_or("open a terrain first")?
            .clone();
        self.stroke = Some(Stroke {
            baseline,
            brush,
            centers: BTreeSet::new(),
            heights: BTreeMap::new(),
            footprint_visits: 0,
            last: brush.center,
        });
        self.update_stroke(brush.center)
    }
    pub fn update_stroke(&mut self, center: [u32; 2]) -> Result<(), String> {
        let mut stroke = self.stroke.take().ok_or("begin a terrain stroke first")?;
        let result = (|| {
            self.scene_base()?;
            let baseline = stroke.baseline.document.terrain();
            if center[0] >= baseline.width() || center[1] >= baseline.depth() {
                return Err("terrain stroke center is outside the grid".into());
            }
            for at in raster_segment(stroke.last, center) {
                if !center_is_surface(baseline, at) {
                    return Err("terrain stroke crosses a hole or outside surface".into());
                }
                if stroke.centers.contains(&at) {
                    continue;
                }
                if stroke.centers.len() >= MAX_STROKE_CENTERS {
                    return Err("terrain stroke center limit".into());
                }
                let edits = TerrainBrush {
                    center: at,
                    ..stroke.brush
                }
                .edits(baseline)?;
                stroke.footprint_visits = stroke
                    .footprint_visits
                    .checked_add(edits.len())
                    .filter(|n| *n <= MAX_STROKE_FOOTPRINT_VISITS)
                    .ok_or("terrain stroke footprint work limit")?;
                for edit in edits {
                    let Edit::SetHeight { x, z, height } = edit else {
                        unreachable!("height brush")
                    };
                    stroke.heights.entry((z, x)).or_insert(height);
                    if stroke.heights.len() > MAX_STROKE_VERTICES {
                        return Err("terrain stroke vertex limit".into());
                    }
                }
                stroke.centers.insert(at);
            }
            let edits: Vec<_> = stroke
                .heights
                .iter()
                .map(|(&(z, x), &height)| Edit::SetHeight { x, z, height })
                .collect();
            let mut document = stroke.baseline.document.clone();
            document.apply(&edits).map_err(|e| e.to_string())?;
            let mut admitted = stroke.baseline.clone();
            admitted.replace_document(document, self.render_admission_error.as_deref())?;
            check_render_admission(&admitted.model, self.render_admission_error.as_deref())?;
            self.admitted = Some(admitted);
            stroke.last = center;
            Ok(())
        })();
        if result.is_ok() {
            self.stroke = Some(stroke);
        } else {
            self.admitted = Some(stroke.baseline);
        }
        result
    }
    pub fn finish_stroke(&mut self) -> Result<(), String> {
        let result = (|| {
            if !self.stroke_active() {
                return Err("begin a terrain stroke first".into());
            }
            self.scene_base()?;
            let admitted = self.admitted.as_ref().ok_or("open a terrain first")?;
            check_render_admission(&admitted.model, self.render_admission_error.as_deref())
        })();
        if result.is_err() {
            self.cancel_stroke();
        } else {
            self.stroke = None;
        }
        result
    }
    pub fn cancel_stroke(&mut self) -> bool {
        let Some(stroke) = self.stroke.take() else {
            return false;
        };
        self.admitted = Some(stroke.baseline);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain_document::{brush::BrushOperation, NewTerrain};
    use std::sync::Arc;

    fn setup(side: u32) -> (tempfile::TempDir, TerrainSession) {
        let dir = tempfile::tempdir().unwrap();
        let scene = dir.path().join("yard.scene.yaml");
        std::fs::write(&scene, "saved scene").unwrap();
        let mut s = TerrainSession::default();
        s.set_scene(Some(&scene)).unwrap();
        s.new_local(
            "terrain.orrt",
            NewTerrain {
                width: side,
                depth: side,
                ..NewTerrain::default()
            },
        )
        .unwrap();
        s.save().unwrap();
        (dir, s)
    }
    fn brush(center: [u32; 2], radius: u32) -> TerrainBrush {
        TerrainBrush {
            center,
            radius,
            operation: BrushOperation::RaiseLower { delta: FP::ONE },
        }
    }
    fn history(document: &orr_terrain::TerrainDocument) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let mut before = document.clone();
        let mut undo = Vec::new();
        while before.undo() {
            undo.push(before.terrain().cook());
        }
        let mut after = document.clone();
        let mut redo = Vec::new();
        while after.redo() {
            redo.push(after.terrain().cook());
        }
        (undo, redo)
    }
    fn assert_restored(s: &TerrainSession, before: &AdmittedTerrain) {
        assert!(!s.stroke_active());
        let after = s.admitted.as_ref().unwrap();
        assert_eq!(after.bytes, before.bytes);
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.saved_bytes, before.saved_bytes);
        assert_eq!(after.source, before.source);
        assert_eq!(after.dirty(), before.dirty());
        assert_eq!(after.query, before.query);
        assert_eq!(after.surface, before.surface);
        assert_eq!(after.marker, before.marker);
        assert_eq!(
            after.document.dirty_region(),
            before.document.dirty_region()
        );
        assert_eq!(history(&after.document), history(&before.document));
        match (&after.model, &before.model) {
            (Some(a), Some(b)) => assert!(Arc::ptr_eq(a, b)),
            (None, None) => {}
            _ => panic!("changed model admission"),
        }
    }
    fn with_redo(s: &mut TerrainSession) {
        s.apply(&[Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::ONE,
        }])
        .unwrap();
        s.save().unwrap();
        s.apply(&[Edit::SetHeight {
            x: 0,
            z: 1,
            height: FP::from_int(2),
        }])
        .unwrap();
        s.undo().unwrap();
        s.set_query(Some([FP::from_int(3), FP::from_int(3)]))
            .unwrap();
    }

    #[test]
    fn sparse_dense_segments_and_overlaps_commit_exactly_one_transaction() {
        let mut results = Vec::new();
        for dense in [false, true] {
            let (_dir, mut s) = setup(9);
            let original = s.bytes().unwrap().to_vec();
            s.begin_stroke(brush([1, 1], 1)).unwrap();
            if dense {
                s.update_stroke([3, 2]).unwrap();
                s.update_stroke([5, 3]).unwrap();
            }
            s.update_stroke([7, 4]).unwrap();
            s.update_stroke([1, 1]).unwrap();
            s.update_stroke([7, 4]).unwrap();
            for _ in 0..8 {
                s.update_stroke([7, 4]).unwrap();
            }
            assert!(!s.can_undo() && !s.can_redo());
            assert!(s.save().is_err() && s.set_query(None).is_err());
            s.finish_stroke().unwrap();
            let edited = s.bytes().unwrap().to_vec();
            assert_ne!(edited, original);
            assert!(s
                .terrain()
                .unwrap()
                .heights()
                .iter()
                .all(|h| *h == FP::ZERO || *h == FP::ONE));
            assert_eq!(s.terrain().unwrap().heights()[4 * 9 + 7], FP::ONE);
            assert!(s.undo().unwrap());
            assert_eq!(s.bytes().unwrap(), original);
            assert!(!s.can_undo());
            assert!(s.redo().unwrap());
            assert_eq!(s.bytes().unwrap(), edited);
            s.save().unwrap();
            s.close().unwrap();
            s.open_local("terrain.orrt").unwrap();
            assert_eq!(s.bytes().unwrap(), edited);
            assert!(!s.dirty());
            results.push(edited);
        }
        assert_eq!(results[0], results[1]);
    }
    #[test]
    fn smooth_reads_stroke_start_and_noop_retains_redo() {
        let (_dir, mut s) = setup(9);
        s.apply(&[Edit::SetHeight {
            x: 4,
            z: 4,
            height: FP::from_int(9),
        }])
        .unwrap();
        s.begin_stroke(TerrainBrush {
            center: [3, 4],
            radius: 0,
            operation: BrushOperation::Smooth,
        })
        .unwrap();
        s.update_stroke([4, 4]).unwrap();
        s.update_stroke([3, 4]).unwrap();
        s.finish_stroke().unwrap();
        assert_eq!(s.terrain().unwrap().heights()[4 * 9 + 3], FP::ONE);
        assert_eq!(s.terrain().unwrap().heights()[4 * 9 + 4], FP::ONE);
        s.undo().unwrap();
        let before = s.admitted.as_ref().unwrap().clone();
        s.begin_stroke(TerrainBrush {
            center: [2, 2],
            radius: 1,
            operation: BrushOperation::RaiseLower { delta: FP::ZERO },
        })
        .unwrap();
        s.update_stroke([6, 2]).unwrap();
        s.finish_stroke().unwrap();
        assert_restored(&s, &before);
        assert!(s.can_redo());
    }
    #[test]
    fn cancellation_overflow_and_admission_restore_saved_dirty_query_and_history() {
        let (dir, mut s) = setup(9);
        with_redo(&mut s);
        for dirty in [false, true] {
            if dirty {
                s.apply(&[Edit::SetHeight {
                    x: 1,
                    z: 0,
                    height: FP::from_int(2),
                }])
                .unwrap();
            }
            let before = s.admitted.as_ref().unwrap().clone();
            let saved = std::fs::read(dir.path().join("terrain.orrt")).unwrap();
            s.begin_stroke(brush([3, 3], 1)).unwrap();
            s.update_stroke([6, 3]).unwrap();
            assert!(s.cancel_stroke());
            assert_restored(&s, &before);
            assert_eq!(
                std::fs::read(dir.path().join("terrain.orrt")).unwrap(),
                saved
            );
            assert!(s
                .begin_stroke(TerrainBrush {
                    center: [0, 0],
                    radius: 0,
                    operation: BrushOperation::RaiseLower {
                        delta: FP::from_raw(i64::MAX)
                    }
                })
                .is_err());
            assert_restored(&s, &before);
            s.begin_stroke(brush([3, 3], 1)).unwrap();
            s.set_render_admission_error(Some("viewport capacity failure".into()));
            assert!(s.update_stroke([4, 3]).is_err());
            assert_restored(&s, &before);
            s.set_render_admission_error(None);
            s.begin_stroke(brush([3, 3], 1)).unwrap();
            assert!(s.update_stroke([u32::MAX, 0]).is_err());
            assert_restored(&s, &before);
        }
    }
    #[test]
    fn sparse_hole_crossing_and_all_work_caps_cancel_the_whole_stroke() {
        let (_dir, mut s) = setup(9);
        s.apply(&[Edit::SetHole {
            x: 4,
            z: 2,
            hole: true,
        }])
        .unwrap();
        let before = s.admitted.as_ref().unwrap().clone();
        s.begin_stroke(brush([2, 2], 0)).unwrap();
        assert!(s.update_stroke([6, 2]).unwrap_err().contains("hole"));
        assert_restored(&s, &before);
        let (_dir, mut s) = setup(129);
        let before = s.admitted.as_ref().unwrap().clone();
        s.begin_stroke(brush([0, 0], 0)).unwrap();
        s.update_stroke([128, 0]).unwrap();
        s.update_stroke([128, 1]).unwrap();
        assert!(s
            .update_stroke([0, 1])
            .unwrap_err()
            .contains("center limit"));
        assert_restored(&s, &before);
        s.begin_stroke(brush([32, 64], 32)).unwrap();
        assert!(s.update_stroke([96, 64]).unwrap_err().contains("limit"));
        assert_restored(&s, &before);
        // Radius 32 while circling nearby centers keeps the union below 4096 but
        // eventually exceeds 65536 footprint visits, independently of that cap.
        s.begin_stroke(brush([64, 64], 32)).unwrap();
        let mut failure = None;
        'path: for z in 64..70 {
            for x in 64..70 {
                if let Err(error) = s.update_stroke([x, z]) {
                    failure = Some(error);
                    break 'path;
                }
            }
        }
        assert!(failure.unwrap().contains("work limit"));
        assert_restored(&s, &before);
    }
}
