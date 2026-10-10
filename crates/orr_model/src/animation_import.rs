//! Strict, bounded glTF 2.0/GLB skeletal-animation import. No network reads.
//!
//! The shared resource parser retains the static importer's image/material and
//! containment checks. This entrypoint adds TRS nodes, four influences, skins,
//! and STEP/LINEAR TRS channels. Morph targets, extra influence sets, matrix
//! nodes, sparse accessors, extensions and CUBICSPLINE fail closed.
//! See https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#skins
//! and https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#animations.
use crate::{
    animation::*,
    import::{self, at, Accessor, AnimationSampler, Document, Loaded, PrimitiveDef},
    *,
};
use std::{collections::BTreeSet, path::Path};

/// Import from an immutable package snapshot, rejecting escaping symlinks/FIFOs.
pub fn import_path(root: &Path, asset_id: &str) -> Result<AnimatedModel, Error> {
    import::import_path_with(root, asset_id, |id, bytes, resolve| {
        import_with_resolver(id, bytes, resolve)
    })
}

/// URI resolver receives only normalized relative paths. Embedded GLB/data URI
/// resources do not call it; external resources are cached, hashed and bounded.
pub fn import_with_resolver(
    asset_id: &str,
    bytes: &[u8],
    resolver: impl FnMut(&str) -> Result<Vec<u8>, Error>,
) -> Result<AnimatedModel, Error> {
    let Loaded {
        doc,
        buffers,
        dependencies,
        materials,
        images,
    } = import::load_resources(asset_id, bytes, resolver, true)?;
    validate_document_limits(&doc)?;
    let (nodes, parents, active) = hierarchy(&doc)?;
    let skins = read_skins(&doc, &buffers, &parents, &active)?;
    let clips = read_clips(&doc, &buffers)?;
    let primitives = read_primitives(&doc, &buffers, asset_id, &active)?;
    AnimatedModel::new(AnimatedSource {
        format: "orr_animated_model".into(),
        version: 1,
        asset_id: asset_id.into(),
        dependencies,
        materials,
        images,
        nodes,
        skins,
        primitives,
        clips,
    })
}

// Called before resolving/decoding any resources as well as at this entrypoint.
pub(crate) fn validate_document_limits(doc: &Document) -> Result<(), Error> {
    if doc.skins.as_ref().is_some_and(Vec::is_empty)
        || doc.animations.as_ref().is_some_and(Vec::is_empty)
    {
        return Err(invalid("present skins/animations must not be empty"));
    }
    let skins = doc.skins.as_deref().unwrap_or_default();
    let clips = doc.animations.as_deref().unwrap_or_default();
    if skins.len() > MAX_SKINS || clips.len() > MAX_CLIPS {
        return Err(invalid("skin/clip collection budget exceeded"));
    }
    for skin in skins {
        if skin.joints.is_empty() || skin.joints.len() > MAX_JOINTS {
            return Err(invalid("skin joint budget exceeded"));
        }
    }
    for clip in clips {
        if clip.channels.is_empty()
            || clip.samplers.is_empty()
            || clip.channels.len() > MAX_CHANNELS
            || clip.samplers.len() > MAX_CHANNELS
        {
            return Err(invalid("animation channel/sampler budget exceeded"));
        }
    }
    Ok(())
}

type Hierarchy = (Vec<AnimatedNode>, Vec<Option<usize>>, Vec<bool>);
fn hierarchy(doc: &Document) -> Result<Hierarchy, Error> {
    let mut parents = vec![None; doc.nodes.len()];
    let mut nodes = Vec::with_capacity(doc.nodes.len());
    for (i, node) in doc.nodes.iter().enumerate() {
        if node.matrix.is_some() {
            return Err(invalid(
                "animated subset requires TRS nodes; matrix nodes unsupported",
            ));
        }
        node.transform()?;
        let rest = Trs {
            translation: node.translation.unwrap_or([0.0; 3]),
            rotation: node.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0]),
            scale: node.scale.unwrap_or([1.0; 3]),
        };
        if rest.scale.iter().any(|s| !s.is_finite() || *s <= 0.0) {
            return Err(invalid("animated node scale must be positive"));
        }
        if let Some(mesh) = node.mesh {
            at(&doc.meshes, mesh, "node mesh")?;
        }
        if let Some(skin) = node.skin {
            at(doc.skins.as_deref().unwrap_or_default(), skin, "node skin")?;
            if node.mesh.is_none() {
                return Err(invalid("skin node requires mesh"));
            }
        }
        for &child in &node.children {
            let parent = parents
                .get_mut(child)
                .ok_or_else(|| invalid("child index out of range"))?;
            if parent.replace(i).is_some() {
                return Err(invalid("duplicate child/multiple parents"));
            }
        }
        nodes.push(AnimatedNode {
            name: node._name.clone().unwrap_or_default(),
            children: node.children.iter().map(|&n| n as u32).collect(),
            rest,
        });
    }
    fn visit(i: usize, doc: &Document, state: &mut [u8], depth: usize) -> Result<(), Error> {
        if depth > import::MAX_DEPTH {
            return Err(invalid("node depth exceeded"));
        }
        if state[i] == 1 {
            return Err(invalid("node cycle"));
        }
        if state[i] == 2 {
            return Ok(());
        }
        state[i] = 1;
        for &child in &doc.nodes[i].children {
            visit(child, doc, state, depth + 1)?;
        }
        state[i] = 2;
        Ok(())
    }
    let mut state = vec![0; nodes.len()];
    // Root-first traversal prevents reverse-indexed unused trees hiding depth.
    for (i, parent) in parents.iter().enumerate() {
        if parent.is_none() {
            visit(i, doc, &mut state, 0)?;
        }
    }
    for i in 0..nodes.len() {
        visit(i, doc, &mut state, 0)?;
    }
    for scene in &doc.scenes {
        let mut seen = BTreeSet::new();
        for &root in &scene.nodes {
            if at(&parents, root, "scene root")?.is_some() || !seen.insert(root) {
                return Err(invalid("invalid/duplicate scene root"));
            }
        }
    }
    let scene = at(&doc.scenes, doc.scene.unwrap_or(0), "default scene")?;
    let mut active = vec![false; nodes.len()];
    let mut pending = scene.nodes.clone();
    while let Some(i) = pending.pop() {
        active[i] = true;
        pending.extend(&doc.nodes[i].children);
    }
    Ok((nodes, parents, active))
}

fn read_skins(
    doc: &Document,
    buffers: &[Vec<u8>],
    parents: &[Option<usize>],
    active: &[bool],
) -> Result<Vec<Skin>, Error> {
    let mut skins = Vec::new();
    for (slot, skin) in doc.skins.as_deref().unwrap_or_default().iter().enumerate() {
        let mut seen = BTreeSet::new();
        let mut root = None;
        for &joint in &skin.joints {
            at(&doc.nodes, joint, "joint node")?;
            if !seen.insert(joint) {
                return Err(invalid("duplicate joint node"));
            }
            let mut ancestor = joint;
            while let Some(parent) = parents[ancestor] {
                ancestor = parent;
            }
            if root
                .replace(ancestor)
                .is_some_and(|previous| previous != ancestor)
            {
                return Err(invalid("skin joints require common ancestor"));
            }
            if let Some(skeleton) = skin.skeleton {
                at(&doc.nodes, skeleton, "skeleton node")?;
                let mut current = Some(joint);
                while current.is_some_and(|i| i != skeleton) {
                    current = parents[current.unwrap()];
                }
                if current.is_none() {
                    return Err(invalid("skeleton is not a joint ancestor"));
                }
            }
        }
        let instantiated = doc
            .nodes
            .iter()
            .enumerate()
            .any(|(i, node)| active[i] && node.skin == Some(slot));
        if instantiated && skin.joints.iter().any(|&joint| !active[joint]) {
            return Err(invalid("skin joints must belong to selected scene"));
        }
        let inverse_bind_matrices = if let Some(index) = skin.inverse_bind_matrices {
            let a = at(&doc.accessors, index, "inverse bind accessor")?;
            data_accessor(doc, a)?;
            // A bounded subset accepts extra matrices permitted by glTF, but
            // validates every one and retains exactly the referenced joint set.
            if a.kind != "MAT4"
                || a.component_type != 5126
                || a.normalized
                || a.count < skin.joints.len()
            {
                return Err(invalid(
                    "inverse bind accessor must be f32 MAT4 with enough matrices",
                ));
            }
            let mut matrices = Vec::with_capacity(skin.joints.len());
            for i in 0..a.count {
                let values = a.floats::<16>(doc, buffers, i);
                let matrix = std::array::from_fn(|c| std::array::from_fn(|r| values[c * 4 + r]));
                normal_matrix(matrix)?;
                if determinant(matrix) <= 0.0 {
                    return Err(invalid("mirrored inverse bind matrices unsupported"));
                }
                if i < skin.joints.len() {
                    matrices.push(matrix);
                }
            }
            matrices
        } else {
            vec![IDENTITY; skin.joints.len()]
        };
        skins.push(Skin {
            name: skin.name.clone().unwrap_or_default(),
            joints: skin.joints.iter().map(|&i| i as u32).collect(),
            inverse_bind_matrices,
        });
    }
    Ok(skins)
}

fn data_accessor(doc: &Document, a: &Accessor) -> Result<(), Error> {
    let view = &doc.buffer_views[a.buffer_view];
    if view.byte_stride.is_some() || view.target.is_some() {
        return Err(invalid(
            "animation/inverse-bind accessor cannot use vertex/index target or stride",
        ));
    }
    Ok(())
}

fn interpolation(sampler: &AnimationSampler) -> Result<Interpolation, Error> {
    match sampler.interpolation.as_deref().unwrap_or("LINEAR") {
        "STEP" => Ok(Interpolation::Step),
        "LINEAR" => Ok(Interpolation::Linear),
        "CUBICSPLINE" => Err(invalid("CUBICSPLINE interpolation unsupported")),
        _ => Err(invalid("unsupported animation interpolation")),
    }
}
fn read_clips(doc: &Document, buffers: &[Vec<u8>]) -> Result<Vec<AnimationClip>, Error> {
    let mut clips = Vec::new();
    let mut aggregate_keys = 0usize;
    let mut aggregate_bytes = 0usize;
    let mut sampler_keys = 0usize;
    for clip in doc.animations.as_deref().unwrap_or_default() {
        // Validate unused samplers too. Unknown output shapes/normalized integer
        // rotations cannot silently sneak in through an unreferenced sampler.
        for sampler in &clip.samplers {
            interpolation(sampler)?;
            let input = at(&doc.accessors, sampler.input, "animation input")?;
            let output = at(&doc.accessors, sampler.output, "animation output")?;
            data_accessor(doc, input)?;
            data_accessor(doc, output)?;
            sampler_keys = sampler_keys.saturating_add(input.count);
            if sampler_keys > MAX_ANIMATION_KEYS
                || input.component_type != 5126
                || input.normalized
                || input.kind != "SCALAR"
                || output.component_type != 5126
                || output.normalized
                || !matches!(output.kind.as_str(), "VEC3" | "VEC4")
                || input.count != output.count
            {
                return Err(invalid(
                    "animation sampler count/type/key budget mismatch; only f32 output supported",
                ));
            }
            if input.min.as_ref().is_none_or(|b| b.len() != 1)
                || input.max.as_ref().is_none_or(|b| b.len() != 1)
            {
                return Err(invalid("animation input bounds required"));
            }
            let mut previous = None;
            for i in 0..input.count {
                let [time] = input.floats::<1>(doc, buffers, i);
                if !time.is_finite()
                    || time < 0.0
                    || previous.is_some_and(|p| time <= p)
                    || time < input.min.as_ref().unwrap()[0]
                    || time > input.max.as_ref().unwrap()[0]
                {
                    return Err(invalid("animation times must be finite, nonnegative, bounded and strictly increasing"));
                }
                previous = Some(time);
                for chunk in output.bytes(doc, buffers, i).chunks_exact(4) {
                    if !f32::from_le_bytes(chunk.try_into().expect("f32 component")).is_finite() {
                        return Err(invalid("animation output must be finite"));
                    }
                }
            }
        }
        let mut targets = BTreeSet::new();
        let mut channels = Vec::with_capacity(clip.channels.len());
        for channel in &clip.channels {
            at(&doc.nodes, channel.target.node, "animation target node")?;
            let sampler = at(&clip.samplers, channel.sampler, "animation sampler")?;
            let input = &doc.accessors[sampler.input];
            let output = &doc.accessors[sampler.output];
            let path = channel.target.path.as_str();
            if !targets.insert((channel.target.node, path)) {
                return Err(invalid("duplicate animation channel target/path"));
            }
            let kind = match path {
                "translation" | "scale" => "VEC3",
                "rotation" => "VEC4",
                "weights" => return Err(invalid("morph animation unsupported")),
                _ => return Err(invalid("unsupported animation target path")),
            };
            aggregate_keys = aggregate_keys.saturating_add(input.count);
            aggregate_bytes =
                aggregate_bytes.saturating_add(input.count.saturating_mul(if kind == "VEC4" {
                    20
                } else {
                    16
                }));
            if aggregate_keys > MAX_ANIMATION_KEYS
                || aggregate_bytes > MAX_ANIMATION_BYTES
                || output.kind != kind
            {
                return Err(invalid(
                    "animation output shape/aggregate key budget mismatch",
                ));
            }
            let values = match path {
                "translation" => ChannelValues::Translation(
                    (0..output.count)
                        .map(|i| output.floats::<3>(doc, buffers, i))
                        .collect(),
                ),
                "scale" => ChannelValues::Scale(
                    (0..output.count)
                        .map(|i| output.floats::<3>(doc, buffers, i))
                        .collect(),
                ),
                _ => ChannelValues::Rotation(
                    (0..output.count)
                        .map(|i| output.floats::<4>(doc, buffers, i))
                        .collect(),
                ),
            };
            channels.push(AnimationChannel {
                node: channel.target.node as u32,
                interpolation: interpolation(sampler)?,
                times: (0..input.count)
                    .map(|i| input.floats::<1>(doc, buffers, i)[0])
                    .collect(),
                values,
            });
        }
        clips.push(AnimationClip {
            name: clip.name.clone().unwrap_or_default(),
            channels,
        });
    }
    Ok(clips)
}

fn validate_primitive(doc: &Document, p: &PrimitiveDef) -> Result<bool, Error> {
    let has_skin = p.attributes.contains_key("JOINTS_0") || p.attributes.contains_key("WEIGHTS_0");
    let expected = if has_skin { 5 } else { 3 };
    if p.mode.unwrap_or(4) != 4
        || p.attributes.len() != expected
        || !["POSITION", "NORMAL", "TEXCOORD_0"]
            .iter()
            .all(|key| p.attributes.contains_key(*key))
        || (has_skin
            && (!p.attributes.contains_key("JOINTS_0") || !p.attributes.contains_key("WEIGHTS_0")))
    {
        return Err(invalid("requires indexed TRIANGLES with POSITION/NORMAL/UV0 and at most JOINTS_0/WEIGHTS_0; additional influences unsupported"));
    }
    at(&doc.materials, p.material, "primitive material")?;
    let pos = at(&doc.accessors, p.attributes["POSITION"], "POSITION")?;
    if pos.count > MAX_VERTICES {
        return Err(invalid("vertex budget exceeded"));
    }
    for (name, kind) in [
        ("POSITION", "VEC3"),
        ("NORMAL", "VEC3"),
        ("TEXCOORD_0", "VEC2"),
    ] {
        let a = at(&doc.accessors, p.attributes[name], name)?;
        if a.component_type != 5126 || a.normalized || a.kind != kind || a.count != pos.count {
            return Err(invalid("vertex attribute type/count mismatch"));
        }
        vertex_accessor(doc, a)?;
    }
    if pos.min.as_ref().is_none_or(|v| v.len() != 3)
        || pos.max.as_ref().is_none_or(|v| v.len() != 3)
    {
        return Err(invalid("POSITION bounds required"));
    }
    if has_skin {
        for name in ["JOINTS_0", "WEIGHTS_0"] {
            let a = at(&doc.accessors, p.attributes[name], name)?;
            vertex_accessor(doc, a)?;
            let valid = if name == "JOINTS_0" {
                matches!(a.component_type, 5121 | 5123) && !a.normalized
            } else {
                (a.component_type == 5126 && !a.normalized)
                    || (matches!(a.component_type, 5121 | 5123) && a.normalized)
            };
            if !valid || a.kind != "VEC4" || a.count != pos.count {
                return Err(invalid(
                    "JOINTS_0/WEIGHTS_0 type/normalization/count mismatch",
                ));
            }
        }
    }
    let idx = at(&doc.accessors, p.indices, "indices")?;
    let view = &doc.buffer_views[idx.buffer_view];
    if idx.kind != "SCALAR"
        || !matches!(idx.component_type, 5121 | 5123 | 5125)
        || idx.normalized
        || idx.count % 3 != 0
        || view.byte_stride.is_some()
        || view.target.is_some_and(|n| n != 34963)
    {
        return Err(invalid("invalid triangle indices"));
    }
    Ok(has_skin)
}
fn vertex_accessor(doc: &Document, a: &Accessor) -> Result<(), Error> {
    let view = &doc.buffer_views[a.buffer_view];
    // Vertex attributes are four-byte aligned, even u8/u16 influence streams.
    let (size, count) = a.layout()?;
    if view.target.is_some_and(|n| n != 34962)
        || (view.byte_offset + a.byte_offset) % 4 != 0
        || view.byte_stride.unwrap_or(size * count) % 4 != 0
    {
        return Err(invalid("vertex attribute target/alignment mismatch"));
    }
    Ok(())
}

fn influences(
    doc: &Document,
    buffers: &[Vec<u8>],
    primitive: &PrimitiveDef,
    i: usize,
    joint_count: usize,
) -> Result<([u32; 4], [f32; 4]), Error> {
    let joint = &doc.accessors[primitive.attributes["JOINTS_0"]];
    let weight = &doc.accessors[primitive.attributes["WEIGHTS_0"]];
    let joint_bytes = joint.bytes(doc, buffers, i);
    let weight_bytes = weight.bytes(doc, buffers, i);
    let joints = std::array::from_fn(|n| {
        if joint.component_type == 5121 {
            u32::from(joint_bytes[n])
        } else {
            u32::from(u16::from_le_bytes(
                joint_bytes[n * 2..n * 2 + 2].try_into().expect("u16 joint"),
            ))
        }
    });
    if matches!(weight.component_type, 5121 | 5123) {
        let (raw_sum, unit): (u32, u32) = if weight.component_type == 5121 {
            (weight_bytes.iter().map(|&v| u32::from(v)).sum(), 255)
        } else {
            (
                weight_bytes
                    .chunks_exact(2)
                    .map(|v| u32::from(u16::from_le_bytes(v.try_into().expect("u16 weight"))))
                    .sum(),
                65535,
            )
        };
        if raw_sum != unit {
            return Err(invalid(
                "normalized integer skin weights must sum exactly to 255/65535",
            ));
        }
    }
    let mut weights: [f32; 4] = std::array::from_fn(|n| match weight.component_type {
        5121 => f32::from(weight_bytes[n]) / 255.0,
        5123 => {
            f32::from(u16::from_le_bytes(
                weight_bytes[n * 2..n * 2 + 2]
                    .try_into()
                    .expect("u16 weight"),
            )) / 65535.0
        }
        _ => f32::from_le_bytes(
            weight_bytes[n * 4..n * 4 + 4]
                .try_into()
                .expect("f32 weight"),
        ),
    });
    if joints.iter().any(|&j| j as usize >= joint_count)
        || weights
            .iter()
            .any(|w| !w.is_finite() || *w < 0.0 || *w > 1.0)
    {
        return Err(invalid(
            "invalid joint index or nonfinite/negative/out-of-range weight",
        ));
    }
    for a in 0..4 {
        for b in a + 1..4 {
            if weights[a] > 0.0 && weights[b] > 0.0 && joints[a] == joints[b] {
                return Err(invalid("duplicate nonzero joint influence"));
            }
        }
    }
    let sum: f32 = weights.iter().sum();
    // Integer weights have already passed their exact raw-sum requirement.
    // Only small floating-point arithmetic error is normalized away.
    let tolerance = 1.0e-4;
    if !sum.is_finite() || sum <= 0.0 || (sum - 1.0).abs() > tolerance {
        return Err(invalid("skin weights must have a nonzero unit sum"));
    }
    for value in &mut weights {
        *value /= sum;
    }
    Ok((joints, weights))
}
fn read_primitives(
    doc: &Document,
    buffers: &[Vec<u8>],
    asset_id: &str,
    active: &[bool],
) -> Result<Vec<AnimatedPrimitive>, Error> {
    let (mut count, mut source_vertices, mut source_indices) = (0usize, 0usize, 0usize);
    for mesh in &doc.meshes {
        count = count.saturating_add(mesh.primitives.len());
        if mesh.primitives.is_empty() || count > MAX_PRIMITIVES {
            return Err(invalid("primitive budget exceeded"));
        }
        for primitive in &mesh.primitives {
            let has_skin = validate_primitive(doc, primitive)?;
            let pos = &doc.accessors[primitive.attributes["POSITION"]];
            let normal = &doc.accessors[primitive.attributes["NORMAL"]];
            let uv = &doc.accessors[primitive.attributes["TEXCOORD_0"]];
            let idx = &doc.accessors[primitive.indices];
            source_vertices = source_vertices.saturating_add(pos.count);
            source_indices = source_indices.saturating_add(idx.count);
            if source_vertices > MAX_VERTICES || source_indices > MAX_INDICES {
                return Err(invalid("source geometry aggregate budget exceeded"));
            }
            // Validate values even for meshes unused by the selected scene.
            for i in 0..pos.count {
                let position = pos.floats::<3>(doc, buffers, i);
                let normal = normal.floats::<3>(doc, buffers, i);
                let uv = uv.floats::<2>(doc, buffers, i);
                if (0..3).any(|c| {
                    !position[c].is_finite()
                        || position[c].abs() > 1.0e6
                        || position[c] < pos.min.as_ref().unwrap()[c]
                        || position[c] > pos.max.as_ref().unwrap()[c]
                }) || !normal.iter().all(|v| v.is_finite())
                    || !(0.99..=1.01).contains(&normal.iter().map(|v| v * v).sum::<f32>())
                    || uv.iter().any(|v| !v.is_finite() || v.abs() > 65536.0)
                {
                    return Err(invalid("invalid source vertex position/normal/UV"));
                }
                if has_skin {
                    influences(doc, buffers, primitive, i, usize::MAX)?;
                }
            }
            for i in 0..idx.count {
                let b = idx.bytes(doc, buffers, i);
                let index = match idx.component_type {
                    5121 => u32::from(b[0]),
                    5123 => u32::from(u16::from_le_bytes(b.try_into().expect("u16 index"))),
                    _ => u32::from_le_bytes(b.try_into().expect("u32 index")),
                };
                if index as usize >= pos.count {
                    return Err(invalid("source index out of range"));
                }
            }
        }
    }
    let (mut vertices_total, mut indices_total) = (0usize, 0usize);
    let mut primitives = Vec::new();
    for (node_id, node) in doc.nodes.iter().enumerate() {
        let Some(mesh_id) = node.mesh else {
            continue;
        };
        let mesh = &doc.meshes[mesh_id];
        for (slot, primitive) in mesh.primitives.iter().enumerate() {
            let has_skin = primitive.attributes.contains_key("JOINTS_0");
            if has_skin != node.skin.is_some() {
                return Err(invalid("node skin and primitive influences must agree"));
            }
            let pos = &doc.accessors[primitive.attributes["POSITION"]];
            let normal = &doc.accessors[primitive.attributes["NORMAL"]];
            let uv = &doc.accessors[primitive.attributes["TEXCOORD_0"]];
            let idx = &doc.accessors[primitive.indices];
            vertices_total = vertices_total.saturating_add(pos.count);
            indices_total = indices_total.saturating_add(idx.count);
            if vertices_total > MAX_VERTICES
                || indices_total > MAX_INDICES
                || primitives.len() >= MAX_PRIMITIVES
            {
                return Err(invalid("expanded animated geometry budget exceeded"));
            }
            // Joint references are validated against every instantiated skin,
            // including nodes outside the chosen scene; no large allocation.
            if !active[node_id] {
                if let Some(skin_id) = node.skin {
                    let count = doc.skins.as_ref().unwrap()[skin_id].joints.len();
                    for i in 0..pos.count {
                        influences(doc, buffers, primitive, i, count)?;
                    }
                }
                continue;
            }
            let mut vertices = Vec::with_capacity(pos.count);
            for i in 0..pos.count {
                let position = pos.floats::<3>(doc, buffers, i);
                if (0..3).any(|c| {
                    !position[c].is_finite()
                        || position[c] < pos.min.as_ref().unwrap()[c]
                        || position[c] > pos.max.as_ref().unwrap()[c]
                }) {
                    return Err(invalid("invalid POSITION bounds/value"));
                }
                let (joints, weights) = if let Some(skin_id) = node.skin {
                    let joint_count = doc.skins.as_ref().unwrap()[skin_id].joints.len();
                    influences(doc, buffers, primitive, i, joint_count)?
                } else {
                    ([0; 4], [1.0, 0.0, 0.0, 0.0])
                };
                vertices.push(SkinnedVertex {
                    vertex: Vertex {
                        position,
                        normal: normal.floats(doc, buffers, i),
                        uv: uv.floats(doc, buffers, i),
                    },
                    joints,
                    weights,
                });
            }
            let mut indices = Vec::with_capacity(idx.count);
            for i in 0..idx.count {
                let bytes = idx.bytes(doc, buffers, i);
                let index = match idx.component_type {
                    5121 => u32::from(bytes[0]),
                    5123 => u32::from(u16::from_le_bytes(bytes.try_into().expect("u16 index"))),
                    _ => u32::from_le_bytes(bytes.try_into().expect("u32 index")),
                };
                if index as usize >= pos.count {
                    return Err(invalid("index out of range"));
                }
                indices.push(index);
            }
            primitives.push(AnimatedPrimitive {
                id: format!("{asset_id}#node={node_id}/mesh={mesh_id}/primitive={slot}"),
                node: node_id as u32,
                skin: node.skin.map(|i| i as u32),
                vertices,
                indices,
                material: primitive.material as u32,
            });
        }
    }
    Ok(primitives)
}
