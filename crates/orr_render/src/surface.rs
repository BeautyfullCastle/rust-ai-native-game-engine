//! Surface shaders: user WGSL that decides how a surface looks, lit by the engine.
//!
//! # The contract
//!
//! A surface shader is one WGSL file. It defines exactly one function:
//!
//! ```wgsl
//! fn surface(in: SurfaceInput) -> Surface {
//!     var s = default_surface(in);
//!     let band = step(0.5, fract(in.world.y * params.stripes));
//!     s.albedo = mix(in.color, params.tint, band);
//!     return s;
//! }
//! ```
//!
//! - `SurfaceInput` gives the world position, the position in the object's
//!   own space (`local`), the normal, the texture coordinates of a model
//!   (`uv`, zero on shapes), the base color, roughness, metallic, emissive
//!   strength and the scene time in seconds.
//! - `Surface` is what the engine lights: `albedo`, `normal`, `roughness`,
//!   `metallic` and `emissive`. `default_surface(in)` returns the look the
//!   object has without a surface shader.
//! - The engine does the rest: the sun and its shadows, ambient light, scene
//!   lights and their layers, tone mapping and output encoding. A surface
//!   shader keeps working when that pipeline changes.
//! - The same file works on the 3D renderer's shapes and, with the `models`
//!   feature, on static and skinned models drawn in an imported scene
//!   ([`crate::ModelRenderer::add_surface`],
//!   `SkinnedModelRenderer::add_surface`). Each is lit by its own lighting.
//!
//! # Parameters
//!
//! Comment lines that start with `// @param` declare values a person sets per
//! entity in the editor. The shader reads them as fields of `params`:
//!
//! ```wgsl
//! // @param tint color = 0.9, 0.3, 0.1
//! // @param stripes float 1..40 = 8
//! ```
//!
//! The kinds are `float`, `vec2`, `vec3`, `vec4` and `color` (three channels
//! from 0 to 1). A range `min..max` is optional; it bounds the editor control.
//! All parameters together hold at most [`MAX_SURFACE_FLOATS`] numbers.
//!
//! # Textures
//!
//! Comment lines that start with `// @texture` declare images a person picks
//! per entity in the editor, at most [`MAX_SURFACE_TEXTURES`]. Each becomes a
//! function of its name that reads the image at texture coordinates, repeating
//! outside 0 to 1, filtered for the size the image has on screen:
//!
//! ```wgsl
//! // @texture grass color
//! // @texture bumps data = 0.5, 0.5, 1, 1
//! fn surface(in: SurfaceInput) -> Surface {
//!     var s = default_surface(in);
//!     s.albedo = grass(in.world.xz * 0.25).rgb;
//!     return s;
//! }
//! ```
//!
//! A `color` image holds sRGB colors and is read as linear color; a `data`
//! image holds numbers from 0 to 1 and is read as they are. The optional
//! four numbers are what the function returns while no image is picked:
//! white for `color` and zero for `data` when they are left out. The filter
//! measures how fast the coordinates change between neighboring pixels, so
//! read textures outside branches that depend on the pixel: inside one, the
//! filter can pick a wrong size along the branch's edges.
//!
//! # Errors
//!
//! [`SurfaceShader::compile`] parses and validates the whole shader on the
//! CPU before any GPU work, against the same engine shader the renderer
//! uses. An error names the line and column in the user's file where the
//! validator gives one, so a broken shader never reaches the GPU.
//!
//! Validation cannot find a shader that never ends (an unbounded `loop`). Such
//! a shader stops the GPU; keep loops bounded.

use std::fmt;

/// Numbers that the parameters of one surface shader can hold together.
pub const MAX_SURFACE_FLOATS: usize = 16;
/// Images one surface shader can read.
pub const MAX_SURFACE_TEXTURES: usize = 4;
/// Largest surface shader source, in bytes.
pub const MAX_SURFACE_SOURCE_BYTES: usize = 64 * 1024;

const CONTRACT: &str = include_str!("surface_contract.wgsl");
/// The engine side of a surface shader on the 3D renderer's shapes.
pub(crate) const SHAPE_GLUE: &str = include_str!("surface_glue.wgsl");
/// The engine side of a surface shader on a static model.
#[cfg(feature = "models")]
pub(crate) const MODEL_GLUE: &str = concat!(include_str!("surface_model_glue.wgsl"), include_str!("surface_model_vertex.wgsl"));
/// The engine side of a surface shader on a skinned model.
#[cfg(feature = "animation")]
pub(crate) const SKINNED_GLUE: &str = concat!(include_str!("surface_model_glue.wgsl"), include_str!("surface_skinned_vertex.wgsl"));

/// A shape's look decided by a surface shader. Index into the shaders added to a renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceId(pub u32);

/// What a parameter holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    Float,
    Vec2,
    Vec3,
    Vec4,
    /// Three channels from 0 to 1, shown as a color.
    Color,
}

impl ParamKind {
    /// Numbers the parameter takes.
    pub const fn numbers(self) -> usize {
        match self {
            Self::Float => 1,
            Self::Vec2 => 2,
            Self::Vec3 | Self::Color => 3,
            Self::Vec4 => 4,
        }
    }

    fn wgsl(self) -> &'static str {
        match self {
            Self::Float => "f32",
            Self::Vec2 => "vec2<f32>",
            Self::Vec3 | Self::Color => "vec3<f32>",
            Self::Vec4 => "vec4<f32>",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "float" => Self::Float,
            "vec2" => Self::Vec2,
            "vec3" => Self::Vec3,
            "vec4" => Self::Vec4,
            "color" => Self::Color,
            _ => return None,
        })
    }
}

/// How a surface texture's pixels are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextureKind {
    /// sRGB colors, read as linear color.
    Color,
    /// Numbers from 0 to 1, read as they are.
    Data,
}

/// One declared texture. Its slot is its place among the declared textures.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceTextureSlot {
    pub name: String,
    pub kind: TextureKind,
    /// What the shader reads while no image is picked.
    pub default: [f32; 4],
}

/// One declared parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceParam {
    pub name: String,
    pub kind: ParamKind,
    /// Index of the first number in the parameter values.
    pub offset: usize,
    /// Bounds of each number, for the editor control and for [`SurfaceShader::clamp`].
    pub min: f32,
    pub max: f32,
    /// The value an entity starts with.
    pub default: [f32; 4],
}

/// A problem in a surface shader, at a line and column of the user's file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderError {
    /// 1-based line; 0 when the problem has no place in the file.
    pub line: u32,
    /// 1-based column in bytes; 0 when the problem has no place in the file.
    pub column: u32,
    pub message: String,
}

impl fmt::Display for ShaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            f.write_str(&self.message)
        } else {
            write!(f, "line {}, column {}: {}", self.line, self.column, self.message)
        }
    }
}

impl std::error::Error for ShaderError {}

fn error(line: usize, column: usize, message: impl Into<String>) -> ShaderError {
    ShaderError { line: line as u32, column: column as u32, message: message.into() }
}

/// A surface shader that passed parsing and validation.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceShader {
    name: String,
    params: Vec<SurfaceParam>,
    textures: Vec<SurfaceTextureSlot>,
    /// Contract, parameter struct and user source: everything after the engine glue.
    body: String,
}

const RESERVED: &[&str] = &["in", "params", "surface", "default_surface", "surface_sampler", "surface_textures_keep", "surface_texture_0", "surface_texture_1", "surface_texture_2", "surface_texture_3", "true", "false", "fn", "let", "var", "struct", "loop", "for", "if", "else", "return"];

fn identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !text.starts_with("__")
        && !RESERVED.contains(&text)
        && !naga::keywords::wgsl::RESERVED.contains(&text)
}

fn parse_params(source: &str) -> Result<Vec<SurfaceParam>, ShaderError> {
    let mut params: Vec<SurfaceParam> = Vec::new();
    let mut used = 0;
    for (index, line) in source.lines().enumerate() {
        let number = index + 1;
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("// @param") else { continue };
        let column = line.len() - trimmed.len() + 1;
        let (declaration, default) = match rest.split_once('=') {
            Some((declaration, default)) => (declaration, Some(default)),
            None => (rest, None),
        };
        let words: Vec<&str> = declaration.split_whitespace().collect();
        let usage = "write `// @param <name> <float|vec2|vec3|vec4|color> [min..max] [= values]`";
        let (name, kind, range) = match words.as_slice() {
            [name, kind] => (*name, *kind, None),
            [name, kind, range] => (*name, *kind, Some(*range)),
            _ => return Err(error(number, column, usage)),
        };
        if !identifier(name) {
            return Err(error(number, column, format!("'{name}' is not a usable parameter name")));
        }
        if params.iter().any(|p| p.name == name) {
            return Err(error(number, column, format!("the parameter '{name}' is declared twice")));
        }
        let kind = ParamKind::parse(kind).ok_or_else(|| error(number, column, format!("'{kind}' is not a parameter kind: {usage}")))?;
        let (min, max) = match range {
            Some(range) => {
                let (low, high) = range.split_once("..").ok_or_else(|| error(number, column, format!("'{range}' is not a range like 0..1")))?;
                let low: f32 = low.parse().map_err(|_| error(number, column, format!("'{low}' is not a number")))?;
                let high: f32 = high.parse().map_err(|_| error(number, column, format!("'{high}' is not a number")))?;
                if !(low.is_finite() && high.is_finite() && low < high) {
                    return Err(error(number, column, format!("the range {range} needs a finite minimum below its maximum")));
                }
                (low, high)
            }
            None if kind == ParamKind::Color => (0.0, 1.0),
            None => (-1.0e6, 1.0e6),
        };
        let mut values = [0.0; 4];
        if let Some(default) = default {
            let parts: Vec<&str> = default.split(',').map(str::trim).collect();
            if parts.len() != kind.numbers() {
                return Err(error(number, column, format!("'{name}' needs {} default numbers, found {}", kind.numbers(), parts.len())));
            }
            for (slot, part) in values.iter_mut().zip(&parts) {
                let value: f32 = part.parse().map_err(|_| error(number, column, format!("'{part}' is not a number")))?;
                if !value.is_finite() || value < min || value > max {
                    return Err(error(number, column, format!("the default {value} of '{name}' is outside {min}..{max}")));
                }
                *slot = value;
            }
        } else {
            let start = 0.0_f32.clamp(min, max);
            values = [start; 4];
        }
        if used + kind.numbers() > MAX_SURFACE_FLOATS {
            return Err(error(number, column, format!("the parameters need more than {MAX_SURFACE_FLOATS} numbers")));
        }
        params.push(SurfaceParam { name: name.to_string(), kind, offset: used, min, max, default: values });
        used += kind.numbers();
    }
    Ok(params)
}

fn parse_textures(source: &str, params: &[SurfaceParam]) -> Result<Vec<SurfaceTextureSlot>, ShaderError> {
    let mut textures: Vec<SurfaceTextureSlot> = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let number = index + 1;
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("// @texture").filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace)) else { continue };
        let column = line.len() - trimmed.len() + 1;
        let (declaration, default) = match rest.split_once('=') {
            Some((declaration, default)) => (declaration, Some(default)),
            None => (rest, None),
        };
        let usage = "write `// @texture <name> <color|data> [= r, g, b, a]`";
        let words: Vec<&str> = declaration.split_whitespace().collect();
        let [name, kind] = words.as_slice() else { return Err(error(number, column, usage)) };
        if !identifier(name) {
            return Err(error(number, column, format!("'{name}' is not a usable texture name")));
        }
        if textures.iter().any(|t| t.name == *name) || params.iter().any(|p| p.name == *name) {
            return Err(error(number, column, format!("the name '{name}' is declared twice")));
        }
        let kind = match *kind {
            "color" => TextureKind::Color,
            "data" => TextureKind::Data,
            other => return Err(error(number, column, format!("'{other}' is not a texture kind: {usage}"))),
        };
        let mut values = match kind {
            TextureKind::Color => [1.0; 4],
            TextureKind::Data => [0.0; 4],
        };
        if let Some(default) = default {
            let parts: Vec<&str> = default.split(',').map(str::trim).collect();
            if parts.len() != 4 {
                return Err(error(number, column, format!("'{name}' needs 4 default numbers, found {}", parts.len())));
            }
            for (slot, part) in values.iter_mut().zip(&parts) {
                let value: f32 = part.parse().map_err(|_| error(number, column, format!("'{part}' is not a number")))?;
                if !(0.0..=1.0).contains(&value) {
                    return Err(error(number, column, format!("the default {value} of '{name}' is outside 0..1")));
                }
                *slot = value;
            }
        }
        if textures.len() == MAX_SURFACE_TEXTURES {
            return Err(error(number, column, format!("a shader reads at most {MAX_SURFACE_TEXTURES} textures")));
        }
        textures.push(SurfaceTextureSlot { name: name.to_string(), kind, default: values });
    }
    Ok(textures)
}

fn params_wgsl(params: &[SurfaceParam], textures: &[SurfaceTextureSlot]) -> String {
    let mut fields = String::new();
    let mut reads = String::new();
    for p in params {
        fields.push_str(&format!("    {}: {},\n", p.name, p.kind.wgsl()));
        let at = |k: usize| format!("surface_raw[{}]", p.offset + k);
        let value = match p.kind {
            ParamKind::Float => at(0),
            ParamKind::Vec2 => format!("vec2<f32>({}, {})", at(0), at(1)),
            ParamKind::Vec3 | ParamKind::Color => format!("vec3<f32>({}, {}, {})", at(0), at(1), at(2)),
            ParamKind::Vec4 => format!("vec4<f32>({}, {}, {}, {})", at(0), at(1), at(2), at(3)),
        };
        reads.push_str(&format!("    p.{} = {value};\n", p.name));
    }
    if params.is_empty() {
        // WGSL has no empty structs.
        fields.push_str("    unused: f32,\n");
    }
    let mut samplers = String::new();
    for (slot, texture) in textures.iter().enumerate() {
        samplers.push_str(&format!("fn {}(uv: vec2<f32>) -> vec4<f32> {{ return textureSample(surface_texture_{slot}, surface_sampler, uv); }}\n", texture.name));
    }
    format!("struct SurfaceParams {{\n{fields}}};\nvar<private> params: SurfaceParams;\nfn surface_params() -> SurfaceParams {{\n    var p: SurfaceParams;\n{reads}    return p;\n}}\n{samplers}")
}

/// The 3D renderer's own shader, with the scene light functions and, with the
/// `irradiance-probes` feature, the probe functions. The renderer builds its
/// pipelines from it, and surface shaders are validated against it.
pub(crate) fn engine_source() -> String {
    // The shapes read the shared map through their own shadow map binding.
    let cascades = crate::SHADOW_CASCADES_WGSL.replace("shared_shadow_map", "shadow_map").replace("shared_shadow_sampler", "shadow_samp");
    let source = crate::scene_lights::shader(include_str!("shader3d.wgsl"), "g", "shadow_samp") + &cascades;
    #[cfg(feature = "irradiance-probes")]
    let source = crate::renderer3d::irradiance::shader(&source, "g");
    source
}

fn deepest_message(error: &dyn std::error::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(next) = source {
        parts.push(next.to_string());
        source = next.source();
    }
    parts.dedup();
    parts.join(": ")
}

/// Parses and validates `full`, a whole module that ends with the user's file
/// of `user_len` bytes. Errors name lines of the user's file.
fn validate(full: &str, user_len: usize) -> Result<naga::Module, ShaderError> {
    // Lines before the user's source, to turn a line of the whole shader into a line of the user's file.
    let before = full.len() - user_len;
    let first_user_line = full[..before].matches('\n').count() + 1;
    let place = |line: u32, column: u32| -> (usize, usize, bool) {
        let line = line as usize;
        if line >= first_user_line {
            (line - first_user_line + 1, column as usize, true)
        } else {
            (0, 0, false)
        }
    };
    let module = naga::front::wgsl::parse_str(full).map_err(|e| {
        // The first label in the user's file, else the first label at all. Some errors carry none.
        let locations: Vec<_> = e.labels().filter(|(span, _)| span.is_defined()).map(|(span, _)| span.location(full)).collect();
        let chosen = locations.iter().find(|l| l.line_number as usize >= first_user_line).or(locations.first());
        let (line, column, in_user) = chosen.map_or((0, 0, false), |l| place(l.line_number, l.line_position));
        let mut message = e.message().to_string();
        // An error inside the engine's part means the user's code does not fit the contract.
        if message.contains("`surface`") || (chosen.is_some() && !in_user) {
            message.push_str(" (the shader must define fn surface(in: SurfaceInput) -> Surface)");
        }
        error(line, column, message)
    })?;
    let mut validator = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty());
    validator.validate(&module).map_err(|e| {
        // The narrowest span is the expression at fault; wider ones are the enclosing function.
        let narrowest = e.spans().filter(|(span, _)| span.is_defined()).map(|(span, _)| span.location(full)).min_by_key(|l| l.length);
        let (line, column, _) = narrowest.map_or((0, 0, false), |l| place(l.line_number, l.line_position));
        error(line, column, deepest_message(e.as_inner()))
    })?;
    Ok(module)
}

impl SurfaceShader {
    /// Reads the parameters and validates the shader against the shape
    /// renderer's own shader and, with the `models` feature, the model
    /// shader. `name` is how the editor and errors refer to it.
    pub fn compile(name: &str, source: &str) -> Result<Self, ShaderError> {
        if source.len() > MAX_SURFACE_SOURCE_BYTES {
            return Err(error(0, 0, format!("the shader is larger than {MAX_SURFACE_SOURCE_BYTES} bytes")));
        }
        let params = parse_params(source)?;
        let textures = parse_textures(source, &params)?;
        let body = format!("{CONTRACT}\n{}\n{source}", params_wgsl(&params, &textures));
        let shader = Self { name: name.to_string(), params, textures, body };
        let module = validate(&shader.source_for(&engine_source(), SHAPE_GLUE), source.len())?;
        // The same file must fit models too, so one material works on both.
        #[cfg(feature = "models")]
        validate(&shader.source_for(&crate::model_renderer::model_engine_source(), MODEL_GLUE), source.len())?;
        #[cfg(feature = "animation")]
        validate(&shader.source_for(&crate::skinned::skinned_engine_source(), SKINNED_GLUE), source.len())?;
        let surface = module.functions.iter().find(|(_, f)| f.name.as_deref() == Some("surface"));
        let expected = |handle: naga::Handle<naga::Type>, name: &str| module.types[handle].name.as_deref() == Some(name);
        match surface {
            Some((_, f)) if f.arguments.len() == 1 && expected(f.arguments[0].ty, "SurfaceInput") && f.result.as_ref().is_some_and(|r| expected(r.ty, "Surface")) => Ok(shader),
            _ => Err(error(0, 0, "the shader must define fn surface(in: SurfaceInput) -> Surface")),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn params(&self) -> &[SurfaceParam] {
        &self.params
    }

    /// The declared textures, in slot order.
    pub fn textures(&self) -> &[SurfaceTextureSlot] {
        &self.textures
    }

    /// The parameter values an entity starts with.
    pub fn default_values(&self) -> [f32; MAX_SURFACE_FLOATS] {
        let mut values = [0.0; MAX_SURFACE_FLOATS];
        for p in &self.params {
            values[p.offset..p.offset + p.kind.numbers()].copy_from_slice(&p.default[..p.kind.numbers()]);
        }
        values
    }

    /// `values` with every declared number inside its range and every unused
    /// or broken number replaced by its default.
    pub fn clamp(&self, values: &[f32; MAX_SURFACE_FLOATS]) -> [f32; MAX_SURFACE_FLOATS] {
        let mut out = self.default_values();
        for p in &self.params {
            for k in 0..p.kind.numbers() {
                let value = values[p.offset + k];
                if value.is_finite() {
                    out[p.offset + k] = value.clamp(p.min, p.max);
                }
            }
        }
        out
    }

    /// The whole WGSL module: the renderer's own shader `engine`, its surface
    /// `glue`, then the contract, the parameters and the user's file.
    pub(crate) fn source_for(&self, engine: &str, glue: &str) -> String {
        format!("{engine}\n{glue}\n{}", self.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRIPES: &str = "// Stripes along the height.
// @param tint color = 0.9, 0.3, 0.1
// @param stripes float 1..40 = 8
// @param offset vec2
fn surface(in: SurfaceInput) -> Surface {
    var s = default_surface(in);
    let band = step(0.5, fract(in.world.y * params.stripes + params.offset.x));
    s.albedo = mix(in.color, params.tint, band);
    return s;
}
";

    #[test]
    fn a_valid_shader_compiles_and_declares_its_parameters() {
        let shader = SurfaceShader::compile("stripes", STRIPES).unwrap();
        assert_eq!(shader.name(), "stripes");
        let params: Vec<_> = shader.params().iter().map(|p| (p.name.as_str(), p.kind, p.offset)).collect();
        assert_eq!(params, [("tint", ParamKind::Color, 0), ("stripes", ParamKind::Float, 3), ("offset", ParamKind::Vec2, 4)]);
        let defaults = shader.default_values();
        assert_eq!(&defaults[..6], &[0.9, 0.3, 0.1, 8.0, 0.0, 0.0]);
        let mut values = defaults;
        values[0] = 7.0;
        values[3] = f32::NAN;
        values[15] = 99.0;
        let clamped = shader.clamp(&values);
        assert_eq!((clamped[0], clamped[3], clamped[15]), (1.0, 8.0, 0.0));
    }

    #[test]
    fn a_shader_without_parameters_compiles() {
        let source = "fn surface(in: SurfaceInput) -> Surface { return default_surface(in); }";
        assert!(SurfaceShader::compile("plain", source).unwrap().params().is_empty());
    }

    #[test]
    fn errors_point_at_the_line_and_column_of_the_users_file() {
        let source = "fn surface(in: SurfaceInput) -> Surface {\n    var s = default_surface(in);\n    s.albedo = undefined_thing;\n    return s;\n}\n";
        let error = SurfaceShader::compile("broken", source).unwrap_err();
        assert_eq!((error.line, error.column), (3, 16), "{error}");
        assert!(error.message.contains("undefined_thing"), "{error}");

        let typed = "fn surface(in: SurfaceInput) -> Surface {\n    var s = default_surface(in);\n    s.roughness = vec3<f32>(1.0);\n    return s;\n}\n";
        let error = SurfaceShader::compile("typed", typed).unwrap_err();
        assert_eq!(error.line, 3, "{error}");
    }

    #[test]
    fn a_missing_or_wrong_surface_function_is_named() {
        for source in ["fn other() {}", "fn surface(x: f32) -> f32 { return x; }"] {
            let error = SurfaceShader::compile("none", source).unwrap_err();
            assert!(error.message.contains("fn surface(in: SurfaceInput) -> Surface"), "{error}");
        }
    }

    #[test]
    fn bad_parameter_declarations_are_reported_on_their_line() {
        let body = "fn surface(in: SurfaceInput) -> Surface { return default_surface(in); }";
        for (declaration, needle) in [
            ("// @param tint colour", "not a parameter kind"),
            ("// @param 9lives float", "not a usable parameter name"),
            ("// @param in float", "not a usable parameter name"),
            ("// @param a float 5..1", "below its maximum"),
            ("// @param a float 0..1 = 3", "outside"),
            ("// @param a vec3 = 1, 2", "needs 3 default numbers"),
            ("// @param a", "write `// @param"),
        ] {
            let source = format!("// header\n{declaration}\n{body}");
            let error = SurfaceShader::compile("bad", &source).unwrap_err();
            assert_eq!(error.line, 2, "{declaration}: {error}");
            assert!(error.message.contains(needle), "{declaration}: {error}");
        }
        let twice = format!("// @param a float\n// @param a float\n{body}");
        assert!(SurfaceShader::compile("twice", &twice).unwrap_err().message.contains("declared twice"));
        let many: String = (0..5).map(|i| format!("// @param p{i} vec4\n")).collect::<String>() + body;
        let error = SurfaceShader::compile("many", &many).unwrap_err();
        assert_eq!(error.line, 5, "{error}");
    }

    #[test]
    fn declared_textures_become_functions_of_their_names() {
        let source = "// @texture grass color\n// @texture bumps data = 0.5, 0.5, 1, 1\nfn surface(in: SurfaceInput) -> Surface {\n    var s = default_surface(in);\n    s.albedo = grass(in.world.xz).rgb * bumps(in.uv).z;\n    return s;\n}\n";
        let shader = SurfaceShader::compile("textured", source).unwrap();
        let slots: Vec<_> = shader.textures().iter().map(|t| (t.name.as_str(), t.kind, t.default)).collect();
        assert_eq!(slots, [("grass", TextureKind::Color, [1.0; 4]), ("bumps", TextureKind::Data, [0.5, 0.5, 1.0, 1.0])]);
    }

    #[test]
    fn bad_texture_declarations_are_reported_on_their_line() {
        let body = "fn surface(in: SurfaceInput) -> Surface { return default_surface(in); }";
        for case in [
            ("// header", "// @texture grass colour", "not a texture kind"),
            ("// header", "// @texture grass", "write `// @texture"),
            ("// header", "// @texture params color", "not a usable texture name"),
            ("// header", "// @texture grass data = 1, 2, 0, 0", "outside 0..1"),
            ("// header", "// @texture grass data = 1, 1", "needs 4 default numbers"),
            ("// @param grass float", "// @texture grass color", "declared twice"),
        ] {
            let source = format!("{}\n{}\n{body}", case.0, case.1);
            let (declaration, needle) = (case.1, case.2);
            let error = SurfaceShader::compile("bad", &source).unwrap_err();
            assert_eq!(error.line, 2, "{declaration}: {error}");
            assert!(error.message.contains(needle), "{declaration}: {error}");
        }
        let many: String = (0..5).map(|i| format!("// @texture t{i} color\n")).collect::<String>() + body;
        assert_eq!(SurfaceShader::compile("many", &many).unwrap_err().line, 5);
    }

    #[test]
    fn oversized_sources_are_refused_before_parsing() {
        let source = "x".repeat(MAX_SURFACE_SOURCE_BYTES + 1);
        assert_eq!(SurfaceShader::compile("big", &source).unwrap_err().line, 0);
    }
}

#[cfg(test)]
mod example_tests {
    #[test]
    fn the_example_shaders_next_to_the_example_scenes_compile() {
        for (name, source) in [
            ("stripes", include_str!("../../../scenes/shaders/stripes.wgsl")),
            ("glow_pulse", include_str!("../../../scenes/shaders/glow_pulse.wgsl")),
            ("rock", include_str!("../../../scenes/shaders/rock.wgsl")),
        ] {
            let shader = super::SurfaceShader::compile(name, source).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!shader.params().is_empty(), "{name}");
        }
    }
}
