//! Opt-in presentation-only scalar metallic/roughness shading.
//!
//! These immutable runtime overrides do not change cooked model formats or glTF
//! import support. Existing constructors select [`MaterialShading::LegacyDiffuse`].
//! Base color stays in the model: factors are linear and its sRGB texture is
//! decoded by the texture format before filtering and shading.

/// Minimum perceptual roughness used for finite rasterized GGX evaluation.
/// Authored zero remains zero in the material; this is an evaluation floor only.
pub const MIN_PERCEPTUAL_ROUGHNESS: f32 = 0.045;

/// Validated scalar inputs; private fields prevent bypassing the finite/range check.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScalarMaterial {
    metallic: f32,
    roughness: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialShadingError {
    InvalidMetallic,
    InvalidRoughness,
    MaterialCount,
}
impl std::fmt::Display for MaterialShadingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidMetallic => "metallic must be finite and in 0..=1",
            Self::InvalidRoughness => "roughness must be finite and in 0..=1",
            Self::MaterialCount => "shading table must have exactly one entry per model material",
        })
    }
}
impl std::error::Error for MaterialShadingError {}

impl ScalarMaterial {
    pub fn new(metallic: f32, roughness: f32) -> Result<Self, MaterialShadingError> {
        if !metallic.is_finite() || !(0.0..=1.0).contains(&metallic) {
            return Err(MaterialShadingError::InvalidMetallic);
        }
        if !roughness.is_finite() || !(0.0..=1.0).contains(&roughness) {
            return Err(MaterialShadingError::InvalidRoughness);
        }
        Ok(Self {
            metallic,
            roughness,
        })
    }
    pub const fn metallic(self) -> f32 {
        self.metallic
    }
    pub const fn roughness(self) -> f32 {
        self.roughness
    }
}

/// Select explicitly per imported material; feature unification never upgrades
/// old assets or constructors to a different shading model.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum MaterialShading {
    #[default]
    LegacyDiffuse,
    MetallicRoughness(ScalarMaterial),
}
impl MaterialShading {
    pub(crate) fn uniform(self) -> [f32; 4] {
        match self {
            Self::LegacyDiffuse => [0.0; 4],
            Self::MetallicRoughness(m) => [m.metallic, m.roughness, 0.0, 1.0],
        }
    }
}

pub(crate) fn prepare_materials(
    count: usize,
    materials: Option<&[MaterialShading]>,
) -> Result<Vec<MaterialShading>, MaterialShadingError> {
    match materials {
        Some(values) if values.len() == count => Ok(values.to_vec()),
        Some(_) => Err(MaterialShadingError::MaterialCount),
        None => Ok(vec![MaterialShading::LegacyDiffuse; count]),
    }
}

/// Expand only explicit hooks. Irradiance expansion may run before or after this.
/// No new binding, texture or pass; feature-off never calls this function.
pub(crate) fn shader(source: &str) -> String {
    fn replace_once(source: String, marker: &str, replacement: &str) -> String {
        assert_eq!(
            source.matches(marker).count(),
            1,
            "material shader hook: {marker}"
        );
        source.replacen(marker, replacement, 1)
    }
    let source = replace_once(
        source.to_owned(),
        "// MATERIAL_GLOBAL",
        "material_eye: vec4<f32>, material_view: vec4<f32>,",
    );
    let source = replace_once(source, "// MATERIAL_OBJECT", "material: vec4<f32>,");
    let source = replace_once(
        source,
        "// MATERIAL_SHADE",
        r#"
    if object.material.w > 0.5 {
        let base = sample_base(in.uv).rgb * object.color.rgb;
        let metal = object.material.x;
        let rough = object.material.y;
        var v = -globals.material_view.xyz;
        if globals.material_eye.w < 0.5 {
            v = mr_normalize(globals.material_eye.xyz - in.world_position);
        }
        let l = -globals.direction.xyz;
        color = mr_brdf(n, v, l, base, metal, rough)
            * globals.sun.xyz * globals.sun.w * max(dot(n, l), 0.0) * visibility;
        // The existing hemisphere/probe signal is approximate diffuse lighting,
        // not specular IBL. Its legacy units are retained; metals receive none.
        color += base * (1.0 - metal) * 0.96 * ambient;
        // Preserve the bounded stylized attenuation; this is not photometric.
        if globals.point_color_intensity.w > 0.0 {
            let delta = globals.point_position_range.xyz - in.world_position;
            let distance = length(delta);
            if distance > 1e-6 {
                let pl = delta / distance;
                let attenuation = max(1.0 - distance / globals.point_position_range.w, 0.0);
                color += mr_brdf(n, v, pl, base, metal, rough)
                    * globals.point_color_intensity.xyz * globals.point_color_intensity.w
                    * max(dot(n, pl), 0.0) * attenuation * attenuation;
            }
        }
    }
"#,
    );
    let source = replace_once(
        source,
        "if globals.params.y>0.5 { color=aces(color); }",
        "if globals.params.y>0.5 { if object.material.w>0.5 { color=mr_aces(color); } else { color=aces(color); } }",
    );
    format!("{}\n{source}", include_str!("material_shading.wgsl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table_is_exact_and_legacy_is_explicit() {
        assert_eq!(
            prepare_materials(2, None).unwrap(),
            vec![MaterialShading::LegacyDiffuse; 2]
        );
        assert_eq!(
            prepare_materials(2, Some(&[])),
            Err(MaterialShadingError::MaterialCount)
        );
        let m = MaterialShading::MetallicRoughness(ScalarMaterial::new(1.0, 0.0).unwrap());
        assert_eq!(prepare_materials(1, Some(&[m])).unwrap(), vec![m]);
        assert_eq!(m.uniform(), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(MaterialShading::LegacyDiffuse.uniform(), [0.0; 4]);
    }
    #[test]
    fn shader_hooks_and_roughness_floor_match() {
        assert!(
            include_str!("material_shading.wgsl")
                .contains(&format!("max(roughness, {MIN_PERCEPTUAL_ROUGHNESS})"))
        );
        for original in [
            include_str!("shader_model.wgsl"),
            include_str!("shader_skinned.wgsl"),
        ] {
            let source = shader(original);
            assert!(!source.contains("// MATERIAL_"));
            assert_eq!(
                source.matches("@binding(").count(),
                original.matches("@binding(").count()
            );
            assert!(source.contains(
                "var color=sample_base(in.uv).rgb*object.color.rgb*(ambient+sun+point);"
            ));
            #[cfg(feature = "irradiance-probes")]
            for combined in [
                crate::renderer3d::irradiance::shader(&source, "globals"),
                shader(&crate::renderer3d::irradiance::shader(original, "globals")),
            ] {
                assert_eq!(combined.matches("fn mr_brdf(").count(), 1);
                assert!(
                    combined.contains("ambient = probe_diffuse(in.world_position, n, ambient);")
                );
                assert!(!combined.contains("// MATERIAL_"));
                assert!(!combined.contains("PROBE_GLOBAL"));
            }
        }
    }
}
