//! Pure Stage-viewport subtree reparent proposals. No panel, history, or I/O.
use orr_sample::authored_ui::{Document, Kind};
use orr_sample::collect_ui::layout_document;

/// A structural no-op must not create a history entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Proposal {
    Unchanged,
    Changed(Document),
}

/// Preserve the selected subtree's rectangles at this logical Stage viewport.
/// New ancestor clipping may change visibility and hit testing. This does not
/// promise equal geometry at other viewport sizes, or apply any document edit.
pub fn propose(
    accepted: &Document,
    source_id: &str,
    target_id: Option<&str>,
    viewport: [u32; 2],
) -> Result<Proposal, String> {
    // Match the bounded Stage preview, not an arbitrary display canvas or DPI.
    if viewport.iter().any(|axis| !(240..=2048).contains(axis)) {
        return Err("Reparent requires a Stage viewport within 240..=2048".into());
    }
    accepted.to_bytes()?;
    let source_index = accepted
        .nodes
        .iter()
        .position(|node| node.id == source_id)
        .ok_or("Reparent source does not exist")?;
    let source = &accepted.nodes[source_index];
    let target_index = target_id
        .map(|id| {
            accepted
                .nodes
                .iter()
                .position(|node| node.id == id)
                .ok_or("Reparent target does not exist")
        })
        .transpose()?;
    if let Some(index) = target_index {
        let target = &accepted.nodes[index];
        if index >= source_index {
            return Err(
                "Reparent target must precede source; self and descendant targets are forbidden"
                    .into(),
            );
        }
        if !matches!(target.kind, Kind::Container) || target.screen != source.screen {
            return Err("Reparent target must be a container on the source screen".into());
        }
    }
    if source.parent.as_deref() == target_id {
        return Ok(Proposal::Unchanged);
    }

    // Integer thousandths represent authored coordinates exactly, avoiding a
    // rounded f32 value accidentally admitting a fractional authored offset.
    // Validation bounds nodes/depth/extents, so these i64 operations cannot overflow.
    let mut minima: Vec<[i64; 2]> = Vec::with_capacity(accepted.nodes.len());
    for (index, node) in accepted.nodes.iter().enumerate() {
        let parent = node.parent.as_ref().map(|id| {
            accepted.nodes[..index]
                .iter()
                .position(|node| node.id == *id)
                .expect("validated parent precedes child")
        });
        let min = std::array::from_fn(|axis| {
            let (origin, extent) = parent.map_or((0, i64::from(viewport[axis])), |parent| {
                (
                    minima[parent][axis],
                    i64::from(accepted.nodes[parent].size[axis]),
                )
            });
            origin + extent * i64::from(node.anchor[axis]) + 1000 * i64::from(node.offset[axis])
        });
        minima.push(min);
    }
    let mut offset = [0i16; 2];
    for (axis, output) in offset.iter_mut().enumerate() {
        let (origin, extent) = target_index.map_or((0, i64::from(viewport[axis])), |parent| {
            (
                minima[parent][axis],
                i64::from(accepted.nodes[parent].size[axis]),
            )
        });
        let thousandths =
            minima[source_index][axis] - origin - extent * i64::from(source.anchor[axis]);
        if thousandths % 1000 != 0 {
            return Err(
                "Reparent requires a fractional offset; exact integer preservation is impossible"
                    .into(),
            );
        }
        let value = thousandths / 1000;
        if !(-4096..=4096).contains(&value) {
            return Err("Reparent offset exceeds -4096..=4096".into());
        }
        *output = value as i16;
    }
    let mut candidate = accepted.clone();
    candidate.nodes[source_index].parent = target_id.map(str::to_owned);
    candidate.nodes[source_index].offset = offset;
    candidate.to_bytes()?;
    let bounds = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(viewport[0] as f32, viewport[1] as f32),
    );
    let before = layout_document(accepted, source.screen, bounds)?;
    let after = layout_document(&candidate, source.screen, bounds)?;
    // Production f32 grouping can differ even when exact authored coordinates
    // agree. Fail closed rather than accepting even a subpixel displacement.
    // Compare every node, including interleaved descendants, but not clipping.
    if before.len() != after.len()
        || before
            .iter()
            .zip(&after)
            .any(|(a, b)| a.index() != b.index() || a.rect() != b.rect())
    {
        return Err("Reparent cannot preserve the exact production layout at this viewport".into());
    }
    Ok(Proposal::Changed(candidate))
}
