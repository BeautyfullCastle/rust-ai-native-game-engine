//! One explicit default-scene subtree from one immutable, self-contained GLB.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GlbSubtreeVariant {
    pub node_index: u32,
    /// Logical GLB source identity retained in primitive IDs and provenance.
    pub asset_id: String,
}

#[derive(Clone, Debug)]
pub(crate) struct GlbSubtreeCatalog {
    pub source_sha256: String,
    pub nodes: Vec<orr_model::extract::NodeInfo>,
}

/// Inspect one captured, bounded file without opening any license or destination.
pub(crate) fn inspect_glb_subtree_source(path: &Path) -> Result<GlbSubtreeCatalog, String> {
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("glb"))
    {
        return Err("GLB subtree inspection requires a .glb file".into());
    }
    let path = checked_file_path(path, "GLB subtree source")?;
    let input = read_bounded_file(&path, MAX_SOURCE_BYTES, "GLB subtree source")?;
    let nodes = orr_model::extract::list_glb_nodes(&input.bytes, "model.glb")?;
    Ok(GlbSubtreeCatalog {
        source_sha256: input.digest,
        nodes,
    })
}

pub(crate) fn validate_glb_subtree_variant(variant: &GlbSubtreeVariant) -> Result<(), String> {
    validate_variant_identity(&variant.asset_id, ".glb")?;
    if variant.node_index as u64 >= orr_model::MAX_NODES as u64 {
        return Err("GLB subtree node index exceeds the model node limit".into());
    }
    Ok(())
}

pub(crate) fn import_glb_subtree_variant(
    request: Request,
    variant: GlbSubtreeVariant,
) -> Result<Report, String> {
    import_glb_subtree_variant_checked(request, variant, None)
}

pub(crate) fn import_glb_subtree_variant_checked(
    request: Request,
    variant: GlbSubtreeVariant,
    expected_source_sha256: Option<String>,
) -> Result<Report, String> {
    validate_expected_source_hash(expected_source_sha256.as_deref())?;
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    if request.source_format != SourceFormat::RawGlb || request.kind != ModelKind::Static {
        return Err("GLB subtree requires RawGlb source and Static model mode".into());
    }
    validate_glb_subtree_variant(&variant)?;
    import_package_inner(
        request,
        Some(AuthoringVariant::GlbSubtree(variant)),
        expected_source_sha256,
    )
}

pub(super) fn glb_subtree_variant_bytes(
    source: &[u8],
    variant: &GlbSubtreeVariant,
) -> Result<Vec<u8>, String> {
    validate_glb_subtree_variant(variant)?;
    let model =
        orr_model::extract::extract_glb_node(source, &variant.asset_id, variant.node_index)?;
    model
        .to_bytes()
        .map_err(|error| format!("GLB subtree cook: {error}"))
}

#[cfg(test)]
#[path = "glb_subtree_tests.rs"]
mod tests;
