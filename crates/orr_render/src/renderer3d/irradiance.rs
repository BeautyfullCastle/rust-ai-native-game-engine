//! Feature-only shader expansion. Existing bindings and their pipeline-specific
//! layouts stay unchanged: the bounded grid is replicated in each renderer's
//! globals, never per draw or per instance. Each allocation grows by 9280 bytes
//! (two for Renderer3D, one for a static/skinned asset). No new GPU objects or
//! cached variants are created by on/off/grid changes. Feature-off uses the
//! original shader source and uniform layout verbatim.

pub(crate) fn shader(source: &str, global: &str) -> String {
    let source = source.replace("let ambient=", "var ambient=");
    let source = source.replace("// IRRADIANCE_GLOBAL", "irradiance: IrradianceUniform,");
    let source = source.replace(
        "// IRRADIANCE_PROCEDURAL",
        "let indirect = probe_diffuse(in.world, n, hemi);",
    );
    let source = source.replace(
        "diffuse_color * (radiance + hemi)",
        "diffuse_color * (radiance + indirect)",
    );
    let source = source.replace(
        "// IRRADIANCE_MODEL",
        "ambient = probe_diffuse(in.world_position, n, ambient);",
    );
    let common = include_str!("irradiance.wgsl").replace("PROBE_GLOBAL", global);
    format!("{common}\n{source}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_preserves_binding_and_output_contracts() {
        for (source, global) in [
            (include_str!("../shader3d.wgsl"), "g"),
            (include_str!("../shader_model.wgsl"), "globals"),
            (include_str!("../shader_skinned.wgsl"), "globals"),
        ] {
            let expanded = shader(source, global);
            assert_eq!(
                expanded.matches("@binding(").count(),
                source.matches("@binding(").count()
            );
            assert!(expanded.contains("irradiance: IrradianceUniform,"));
            assert!(!expanded.contains("PROBE_GLOBAL"));
            assert!(expanded.contains("65504.0"));
        }
        let procedural = shader(include_str!("../shader3d.wgsl"), "g");
        assert!(procedural.contains("hemi * f0 * 0.35 * (1.0 - rough)"));
        let lines = procedural.split("// ---- debug lines ----").nth(1).unwrap();
        assert!(!lines.contains("probe_diffuse"));
    }
}
