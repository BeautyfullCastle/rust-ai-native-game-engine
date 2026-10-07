//! Frame-owned, revision-pinned heightfield assets and atomic admission.

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame, FrameList};
use orr_fp::{fp, FP};
use orr_physics3d::{Body, Collider};
use orr_terrain::Terrain;
use sha2::{Digest, Sha256};
use std::fmt;

/// Maximum vertices on either side of a collision heightfield (4096 cells).
pub const MAX_COLLISION_SIDE: u32 = 65;
/// Absolute world-coordinate and height bound for the collision profile.
pub const MAX_TERRAIN_COORDINATE: FP = fp!(1024);
/// Minimum cell spacing in the collision profile.
pub const MIN_TERRAIN_SPACING: FP = fp!(0.25);
/// Maximum cell spacing in the collision profile.
pub const MAX_TERRAIN_SPACING: FP = fp!(16);
/// Maximum height difference between axis-adjacent samples.
pub const MAX_ADJACENT_HEIGHT_DELTA: FP = fp!(16);

/// One row-major height sample owned by a frame list.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct TerrainHeight(pub FP);

/// One row-major cell hole flag, always canonical 0 or 1.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct TerrainHole(pub u8);

/// One byte of the canonical asset identity.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct TerrainIdentityByte(pub u8);

/// The single collision asset in the frame. All authoritative samples and
/// identity bytes live in the referenced frame lists; no external asset store
/// or process-local cache participates in simulation.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct TerrainAsset {
    /// Full SHA-256 of the canonical `orr_terrain` bytes.
    pub revision: [u8; 32],
    /// World-space X/Z origin. Terrain is always world-aligned and static.
    pub origin: [FP; 2],
    /// Shared positive X/Z cell spacing.
    pub spacing: FP,
    /// Row-major vertex heights, including samples bordering holes.
    pub heights: FrameList<TerrainHeight>,
    /// Row-major cell hole mask, including all cells.
    pub holes: FrameList<TerrainHole>,
    /// Complete asset identity, included in the canonical revision.
    pub identity: FrameList<TerrainIdentityByte>,
    /// Live entity carrying the one [`HeightfieldCollider`].
    pub entity: Entity,
    /// Vertex count along X.
    pub width: u32,
    /// Vertex count along Z.
    pub depth: u32,
    /// 0 for the untouched zero singleton; 1 for an admitted asset.
    pub present: u32,
    /// Explicit padding, always zero.
    pub _pad: u32,
}

/// An explicitly materialized static terrain collider. This component never
/// accompanies a convex [`Body`] or [`Collider`]; the solver receives a derived
/// static contact object rather than a fake convex shape.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct HeightfieldCollider {
    /// Exact asset reference: every one of the SHA-256 bytes must match.
    pub revision: [u8; 32],
    /// Coulomb friction coefficient in [0, 100].
    pub friction: FP,
    /// Restitution in [0, 1].
    pub restitution: FP,
    /// Collision layer bits.
    pub layer: u32,
    /// Layers this collider can touch (both colliders must accept the pair).
    pub mask: u32,
}

/// Rejected admission or invalid authoritative frame data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetError {
    /// Register this crate's types before admission or validation.
    Unregistered,
    /// Bytes do not encode a canonical `orr_terrain` asset.
    InvalidCanonicalBytes,
    /// The expected, collider, or computed full revision differs.
    RevisionMismatch,
    /// Only one asset may be admitted, once, in this bounded stage.
    AlreadyAdmitted,
    /// Metadata, list handles, collider count, or entity references are invalid.
    InvalidFrameState,
    /// Coordinates, dimensions, spacing, or height differences exceed bounds.
    OutsideCollisionProfile,
    /// Material values exceed the solver's supported range.
    InvalidMaterial,
    /// A convex body/shape is attached to the terrain collider entity.
    AmbiguousTerrainBody,
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unregistered => "terrain collision types are not registered",
            Self::InvalidCanonicalBytes => "invalid canonical terrain bytes",
            Self::RevisionMismatch => "full terrain SHA-256 revision mismatch",
            Self::AlreadyAdmitted => "only one static terrain asset may be admitted",
            Self::InvalidFrameState => "invalid frame-owned terrain state",
            Self::OutsideCollisionProfile => "terrain exceeds the bounded collision profile",
            Self::InvalidMaterial => "invalid terrain collider material",
            Self::AmbiguousTerrainBody => "terrain entity must not have a convex body or collider",
        })
    }
}
impl std::error::Error for AssetError {}

/// Register only the terrain integration types; core ECS has no terrain
/// dependency. Register `orr_physics3d` separately when using the solver.
pub fn register(builder: &mut ComponentRegistryBuilder) {
    builder.register_component::<HeightfieldCollider>("orr_terrain_physics3d::HeightfieldCollider");
    builder.register_singleton::<TerrainAsset>("orr_terrain_physics3d::TerrainAsset");
    builder.register_list::<TerrainHeight>("orr_terrain_physics3d::TerrainHeight");
    builder.register_list::<TerrainHole>("orr_terrain_physics3d::TerrainHole");
    builder.register_list::<TerrainIdentityByte>("orr_terrain_physics3d::TerrainIdentityByte");
}

fn check_registered(frame: &Frame) -> Result<(), AssetError> {
    let registry = frame.registry();
    if registry.component_id::<HeightfieldCollider>().is_none()
        || registry.singleton_id::<TerrainAsset>().is_none()
        || registry.list_id::<TerrainHeight>().is_none()
        || registry.list_id::<TerrainHole>().is_none()
        || registry.list_id::<TerrainIdentityByte>().is_none()
    {
        return Err(AssetError::Unregistered);
    }
    Ok(())
}

fn check_empty(frame: &Frame) -> Result<(), AssetError> {
    let asset = frame.singleton::<TerrainAsset>();
    if asset.present != 0 || !frame.dense::<HeightfieldCollider>().0.is_empty() {
        return Err(AssetError::AlreadyAdmitted);
    }
    if bytemuck::bytes_of(asset).iter().any(|&byte| byte != 0) {
        return Err(AssetError::InvalidFrameState);
    }
    Ok(())
}

fn validate_material(collider: &HeightfieldCollider) -> Result<(), AssetError> {
    if collider.friction < FP::ZERO
        || collider.friction > fp!(100)
        || collider.restitution < FP::ZERO
        || collider.restitution > FP::ONE
    {
        return Err(AssetError::InvalidMaterial);
    }
    Ok(())
}

fn validate_profile(
    width: u32,
    depth: u32,
    origin: [FP; 2],
    spacing: FP,
    heights: &[FP],
) -> Result<(), AssetError> {
    let bound = i128::from(MAX_TERRAIN_COORDINATE.raw());
    let within = |raw: i128| (-bound..=bound).contains(&raw);
    if !(2..=MAX_COLLISION_SIDE).contains(&width)
        || !(2..=MAX_COLLISION_SIDE).contains(&depth)
        || !(MIN_TERRAIN_SPACING..=MAX_TERRAIN_SPACING).contains(&spacing)
        || heights.len() != (width * depth) as usize
    {
        return Err(AssetError::OutsideCollisionProfile);
    }
    for (start, count) in origin.into_iter().zip([width, depth]) {
        let start = i128::from(start.raw());
        let end = start + i128::from(count - 1) * i128::from(spacing.raw());
        if !within(start) || !within(end) {
            return Err(AssetError::OutsideCollisionProfile);
        }
    }
    let max_delta = i128::from(MAX_ADJACENT_HEIGHT_DELTA.raw());
    for z in 0..depth {
        for x in 0..width {
            let index = (z * width + x) as usize;
            let height = i128::from(heights[index].raw());
            if !within(height)
                || (x > 0 && (height - i128::from(heights[index - 1].raw())).abs() > max_delta)
                || (z > 0
                    && (height - i128::from(heights[index - width as usize].raw())).abs()
                        > max_delta)
            {
                return Err(AssetError::OutsideCollisionProfile);
            }
        }
    }
    Ok(())
}

/// Admit one canonical, fully pinned collision asset into the frame. On every
/// returned error, the entire frame (including allocation order and checksum)
/// is unchanged. Validation precedes all entity/list allocations. Subsequent
/// calls are rejected; asset replacement belongs to a later explicit API.
pub fn admit_heightfield(
    frame: &mut Frame,
    canonical_bytes: &[u8],
    expected_revision: [u8; 32],
    collider: HeightfieldCollider,
) -> Result<Entity, AssetError> {
    check_registered(frame)?;
    check_empty(frame)?;
    validate_material(&collider)?;
    let terrain = Terrain::load(canonical_bytes).map_err(|_| AssetError::InvalidCanonicalBytes)?;
    if terrain.cook() != canonical_bytes {
        return Err(AssetError::InvalidCanonicalBytes);
    }
    let revision: [u8; 32] = Sha256::digest(canonical_bytes).into();
    if revision != expected_revision || collider.revision != expected_revision {
        return Err(AssetError::RevisionMismatch);
    }
    validate_profile(
        terrain.width(),
        terrain.depth(),
        terrain.origin(),
        terrain.spacing(),
        terrain.heights(),
    )?;

    // No fallible validation or external access beyond this point.
    let heights = frame.alloc_list::<TerrainHeight>();
    let holes = frame.alloc_list::<TerrainHole>();
    let identity = frame.alloc_list::<TerrainIdentityByte>();
    for &height in terrain.heights() {
        frame.list_push(heights, TerrainHeight(height));
    }
    for &hole in terrain.holes() {
        frame.list_push(holes, TerrainHole(u8::from(hole)));
    }
    for byte in terrain.asset_id().bytes() {
        frame.list_push(identity, TerrainIdentityByte(byte));
    }
    let entity = frame.spawn();
    frame.add(entity, collider);
    frame.set_singleton(TerrainAsset {
        revision,
        origin: terrain.origin(),
        spacing: terrain.spacing(),
        heights,
        holes,
        identity,
        entity,
        width: terrain.width(),
        depth: terrain.depth(),
        present: 1,
        _pad: 0,
    });
    Ok(entity)
}

/// Borrowed collision data derived exclusively from a validated frame.
#[derive(Clone, Copy, Debug)]
pub struct TerrainView<'a> {
    /// Canonical scalar metadata and frame-list references.
    pub asset: &'a TerrainAsset,
    /// Explicit material, filtering and full asset reference.
    pub collider: &'a HeightfieldCollider,
    /// All row-major vertex samples.
    pub heights: &'a [TerrainHeight],
    /// All row-major 0/1 cell flags.
    pub holes: &'a [TerrainHole],
    /// Full canonical asset identity.
    pub identity: &'a [TerrainIdentityByte],
}

impl TerrainView<'_> {
    /// Position of a canonical vertex in world coordinates.
    pub fn vertex_position(&self, index: u32) -> Option<[FP; 3]> {
        let height = self.heights.get(index as usize)?.0;
        if self.asset.width == 0 {
            return None;
        }
        let coordinate = |origin: FP, offset: u32| {
            let raw = i128::from(origin.raw())
                + i128::from(offset) * i128::from(self.asset.spacing.raw());
            i64::try_from(raw).ok().map(FP::from_raw)
        };
        Some([
            coordinate(self.asset.origin[0], index % self.asset.width)?,
            height,
            coordinate(self.asset.origin[1], index / self.asset.width)?,
        ])
    }
}

fn valid_identity(bytes: &[u8]) -> bool {
    let Ok(identity) = std::str::from_utf8(bytes) else {
        return false;
    };
    !identity.is_empty()
        && identity.len() <= orr_terrain::MAX_ASSET_ID_BYTES
        && identity
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-/ .".contains(&c))
        && !identity.starts_with('/')
        && !identity
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
}

fn frame_revision(view: &TerrainView<'_>) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"ORRTHF\x01\0");
    hash.update((view.identity.len() as u16).to_le_bytes());
    hash.update(bytemuck::cast_slice::<TerrainIdentityByte, u8>(
        view.identity,
    ));
    hash.update(view.asset.width.to_le_bytes());
    hash.update(view.asset.depth.to_le_bytes());
    for value in [
        view.asset.origin[0],
        view.asset.origin[1],
        view.asset.spacing,
    ] {
        hash.update(value.raw().to_le_bytes());
    }
    for height in view.heights {
        hash.update(height.0.raw().to_le_bytes());
    }
    hash.update(bytemuck::cast_slice::<TerrainHole, u8>(view.holes));
    hash.finalize().into()
}

/// Validate frame metadata, list lengths/content, collision bounds, entity and
/// material/reference consistency, and all 256 revision bits before borrowing
/// geometry. This also validates restored snapshots, which can carry arbitrary
/// Pod bytes. It neither mutates the frame nor consults an external asset store.
pub fn terrain_view(frame: &Frame) -> Result<Option<TerrainView<'_>>, AssetError> {
    check_registered(frame)?;
    let asset = frame.singleton::<TerrainAsset>();
    if asset.present == 0 {
        check_empty(frame)?;
        return Ok(None);
    }
    if asset.present != 1
        || asset._pad != 0
        || !frame.exists(asset.entity)
        || frame.dense::<HeightfieldCollider>().0 != [asset.entity]
        || !(2..=MAX_COLLISION_SIDE).contains(&asset.width)
        || !(2..=MAX_COLLISION_SIDE).contains(&asset.depth)
    {
        return Err(AssetError::InvalidFrameState);
    }
    let collider = frame
        .get::<HeightfieldCollider>(asset.entity)
        .ok_or(AssetError::InvalidFrameState)?;
    if (frame.registry().component_id::<Body>().is_some() && frame.has::<Body>(asset.entity))
        || (frame.registry().component_id::<Collider>().is_some()
            && frame.has::<Collider>(asset.entity))
    {
        return Err(AssetError::AmbiguousTerrainBody);
    }
    validate_material(collider)?;
    if collider.revision != asset.revision {
        return Err(AssetError::RevisionMismatch);
    }
    let view = TerrainView {
        asset,
        collider,
        heights: frame.list(asset.heights),
        holes: frame.list(asset.holes),
        identity: frame.list(asset.identity),
    };
    if view.heights.len() != (asset.width * asset.depth) as usize
        || view.holes.len() != ((asset.width - 1) * (asset.depth - 1)) as usize
        || view.holes.iter().any(|hole| hole.0 > 1)
        || !valid_identity(bytemuck::cast_slice(view.identity))
    {
        return Err(AssetError::InvalidFrameState);
    }
    validate_profile(
        asset.width,
        asset.depth,
        asset.origin,
        asset.spacing,
        bytemuck::cast_slice::<TerrainHeight, FP>(view.heights),
    )?;
    if frame_revision(&view) != asset.revision {
        return Err(AssetError::RevisionMismatch);
    }
    Ok(Some(view))
}

/// Rebuild a validated immutable terrain asset from frame-owned content, for
/// read-only consumers. Runtime collision can use [`terrain_view`] directly
/// without allocating this derived value.
pub fn reconstruct_terrain(frame: &Frame) -> Result<Option<Terrain>, AssetError> {
    let Some(view) = terrain_view(frame)? else {
        return Ok(None);
    };
    let identity = std::str::from_utf8(bytemuck::cast_slice(view.identity))
        .map_err(|_| AssetError::InvalidFrameState)?;
    Terrain::new(
        identity.to_owned(),
        view.asset.width,
        view.asset.depth,
        view.asset.origin,
        view.asset.spacing,
        view.heights.iter().map(|height| height.0).collect(),
        view.holes.iter().map(|hole| hole.0 != 0).collect(),
    )
    .map(Some)
    .map_err(|_| AssetError::InvalidFrameState)
}
