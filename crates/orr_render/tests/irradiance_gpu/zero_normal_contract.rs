//! Exact-zero semantic acceptance with lossless raster preconditions.
use super::*;
use orr_render::orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, Cull, DepthAttachment,
    DepthState, PipelineDesc, Topology, VertexAttr, VertexFormat, VertexLayout, VertexStep,
};

pub(super) struct Observation {
    pub raw: [u32; 3],
    pub normalized: [u32; 3],
    pub world: [u32; 3],
    pub gate: u32,
}

pub(super) fn observe(g: &Wgpu, asset: &StaticModel, cam: &Camera3D, size: (u32, u32)) -> [Observation; 3] {
    let primitive = &asset.source().primitives[0];
    assert_eq!(primitive.transform, IDENTITY);
    assert_eq!(primitive.indices, [0, 1, 2, 0, 2, 3]);
    assert_eq!(primitive.vertices.len(), 4);
    assert_eq!(size, (128, 128));
    // Match the production MR-off irradiance expansion without editing its shader.
    // vs_main and model_world are included byte-for-byte. Only the selected
    // fragment entry differs: compiler decisions may therefore differ too.
    let original_shader = include_str!("../../src/shader_model.wgsl");
    let expanded = original_shader
        .replace("let ambient=", "var ambient=")
        .replace("// IRRADIANCE_GLOBAL", "irradiance: IrradianceUniform,")
        .replace("// IRRADIANCE_MODEL", "ambient = probe_diffuse(in.world_position, n, ambient);");
    let common = include_str!("../../src/renderer3d/irradiance.wgsl")
        .replace("PROBE_GLOBAL", "globals");
    let source = format!("{common}\n{expanded}\n{}", r#"
struct Diagnostic { mode: u32, axis: u32, sentinel: u32, padding: u32 };
@group(0) @binding(3) var<uniform> diag_config: Diagnostic;
@fragment fn fs_diagnostic(in: Varying) -> @location(0) vec4<f32> {
    let n = in.normal * inverseSqrt(max(dot(in.normal, in.normal), 1e-20));
    var value = in.normal[diag_config.axis];
    if diag_config.mode == 1u { value = n[diag_config.axis]; }
    if diag_config.mode == 2u {
        let normal_scale = max(abs(n.x), max(abs(n.y), abs(n.z)));
        value = select(0.0, 1.0, all(abs(n) <= vec3<f32>(3.402823e38)) && normal_scale > 0.0);
    }
    if diag_config.mode == 3u { value = in.world_position[diag_config.axis]; }
    var bits = bitcast<u32>(value);
    // Integer sentinels verify every byte without converting through float/FP16.
    if diag_config.mode == 4u { bits = diag_config.sentinel; }
    let bytes = vec4<u32>(bits & 255u, (bits >> 8u) & 255u,
                         (bits >> 16u) & 255u, (bits >> 24u) & 255u);
    return vec4<f32>(bytes) / 255.0;
}
"#);
    let shader = g.create_shader("zero-normal diagnostic", &source);
    let attrs = [
        VertexAttr { location: 0, format: VertexFormat::Float32x3, offset: 0 },
        VertexAttr { location: 1, format: VertexFormat::Float32x3, offset: 12 },
        VertexAttr { location: 2, format: VertexFormat::Float32x2, offset: 24 },
    ];
    let pipeline = g.create_pipeline(&PipelineDesc::<Wgpu> {
        label: "zero-normal diagnostic", shader: &shader,
        vs_entry: "vs_main", fs_entry: "fs_diagnostic",
        vertex_buffers: &[VertexLayout { stride: 32, step: VertexStep::Vertex, attrs: &attrs }],
        color_format: Some(TextureFormat::Rgba8Unorm), blend: Blend::Opaque,
        topology: Topology::TriangleList, cull: Cull::Back,
        depth: Some(DepthState::opaque(TextureFormat::Depth32Float)), samples: 1,
    });
    let buffer = |label: &str, bytes: &[u8], usage: BufferUsage| {
        let buffer = g.create_buffer(&BufferDesc {
            label, size: bytes.len() as u64, usage: usage | BufferUsage::COPY_DST,
        });
        g.write_buffer(&buffer, 0, bytes);
        buffer
    };
    // First64 bytes are the exact production camera view-projection matrix.
    // Remaining globals are zero because this diagnostic does not shade light.
    let mut globals = vec![0_u8; 9536];
    let matrix = cam.view_proj(size.0 as f32 / size.1 as f32).0;
    globals[..64].copy_from_slice(bytemuck::bytes_of(&matrix));
    let globals = buffer("diagnostic globals", &globals, BufferUsage::UNIFORM);
    let mut object = vec![0_u8; 160];
    object[..64].copy_from_slice(bytemuck::bytes_of(&IDENTITY));
    object[64..128].copy_from_slice(bytemuck::bytes_of(&IDENTITY));
    let object = buffer("diagnostic object", &object, BufferUsage::UNIFORM);
    let selection = buffer("diagnostic selection", &[0_u8; 16], BufferUsage::UNIFORM);
    let vertices: Vec<[f32; 8]> = primitive.vertices.iter().map(|v| [
        v.position[0], v.position[1], v.position[2],
        v.normal[0], v.normal[1], v.normal[2], v.uv[0], v.uv[1],
    ]).collect();
    let vertices = buffer("diagnostic vertices", bytemuck::cast_slice(&vertices), BufferUsage::VERTEX);
    let indices = buffer("diagnostic indices", bytemuck::cast_slice(&primitive.indices), BufferUsage::INDEX);
    let bind = g.create_bind_group(&pipeline, 0, &[
        Binding::Uniform { binding: 0, buffer: &globals },
        Binding::Uniform { binding: 2, buffer: &object },
        Binding::Uniform { binding: 3, buffer: &selection },
    ]);
    let color = target(g, TextureFormat::Rgba8Unorm, size);
    let color_view = g.create_texture_view(&color, None);
    let depth = g.create_texture(&TextureDesc {
        label: "diagnostic depth", width: size.0, height: size.1,
        format: TextureFormat::Depth32Float, usage: TextureUsage::RENDER_ATTACHMENT,
        sample_count: 1, view_formats: &[],
    });
    let depth_view = g.create_texture_view(&depth, None);
    let sample = |mode: u32, axis: u32, sentinel: u32| -> [u32; 3] {
        g.write_buffer(&selection, 0, bytemuck::cast_slice(&[mode, axis, sentinel, 0]));
        let mut encoder = g.create_encoder("diagnostic observation");
        g.encode_pass(&mut encoder, "diagnostic observation",
            Some(&ColorAttachment { view: &color_view, clear: Some([0.0; 4]), resolve: None }),
            Some(&DepthAttachment { view: &depth_view, clear: Some(1.0), store: true }),
            &[
                Command::SetPipeline(&pipeline), Command::SetBindGroup(0, &bind),
                Command::SetVertexBuffer(0, &vertices), Command::SetIndexBuffer(&indices),
                Command::DrawIndexed { indices: 0..6, base_vertex: 0, instances: 0..1 },
            ]);
        g.submit(encoder);
        let pixels = g.read_texture(&color);
        assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
        [54_u32, 64, 74].map(|x| {
            let offset = ((60 * size.0 + x) * 4) as usize;
            u32::from_le_bytes(pixels[offset..offset + 4].try_into().unwrap())
        })
    };
    for bits in [0x0000_0000_u32, 0x8000_0000, 0x3f80_0000, 0x33ab_cdef, 0x0000_0001, 0xffff_ffff] {
        assert_eq!(sample(4, 0, bits), [bits; 3], "lossless bit transport and covered pixels");
    }
    let raw: [[u32; 3]; 3] = std::array::from_fn(|axis| sample(0, axis as u32, 0));
    let normalized: [[u32; 3]; 3] = std::array::from_fn(|axis| sample(1, axis as u32, 0));
    let world: [[u32; 3]; 3] = std::array::from_fn(|axis| sample(3, axis as u32, 0));
    let gate = sample(2, 0, 0);
    std::array::from_fn(|index| {
        let observation = Observation {
            raw: raw.map(|axis| axis[index]),
            normalized: normalized.map(|axis| axis[index]),
            world: world.map(|axis| axis[index]),
            gate: gate[index],
        };
        assert!(observation.raw.into_iter().chain(observation.normalized).chain(observation.world)
            .map(f32::from_bits).all(f32::is_finite));
        eprintln!("ZERO_CONTRACT_RASTER pixel=({},60) raw_bits={:08x?} normalized_bits={:08x?} world_bits={:08x?} gate={}",
            [54, 64, 74][index], observation.raw, observation.normalized,
            observation.world, f32::from_bits(observation.gate));
        observation
    })
}

#[test]
fn production_probe_helper_preserves_exact_zero_and_samples_tiny_nonzero_normals() {
    let (_lock, g) = gpu();
    let common = include_str!("../../src/renderer3d/irradiance.wgsl")
        .replace("PROBE_GLOBAL", "globals");
    let source = format!("{common}\n{}", r#"
struct ProbeGlobals { irradiance: IrradianceUniform };
struct ProbeInput { normal: vec4<f32>, fallback: vec4<f32> };
@group(0) @binding(0) var<uniform> globals: ProbeGlobals;
@group(0) @binding(1) var<uniform> probe_input: ProbeInput;
@vertex fn vs_probe(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(p[index], 0.0, 1.0);
}
@fragment fn fs_probe() -> @location(0) vec4<f32> {
    return vec4<f32>(probe_diffuse(vec3<f32>(0.0), probe_input.normal.xyz, probe_input.fallback.xyz), 1.0);
}
"#);
    let shader = g.create_shader("actual probe helper zero contract", &source);
    let pipeline = g.create_pipeline(&PipelineDesc::color(
        "actual probe helper zero contract", &shader, ("vs_probe", "fs_probe"),
        &[], TextureFormat::Rgba16Float, Blend::Opaque,
    ));
    // Packing is only input setup. The expected radiance is the independently
    // authored constant SH0 value, with exactly one cosine/pi conversion.
    let grid = constant_grid([2.0, 3.0, 4.0]);
    let packed = grid.packed_uniform().unwrap();
    let globals = g.create_buffer(&BufferDesc {
        label: "probe helper grid", size: std::mem::size_of_val(&packed) as u64,
        usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
    });
    g.write_buffer(&globals, 0, bytemuck::bytes_of(&packed));
    let input = g.create_buffer(&BufferDesc {
        label: "probe helper exact inputs", size: 32,
        usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
    });
    let bind = g.create_bind_group(&pipeline, 0, &[
        Binding::Uniform { binding: 0, buffer: &globals },
        Binding::Uniform { binding: 1, buffer: &input },
    ]);
    let color = target(&g, TextureFormat::Rgba16Float, (1, 1));
    let view = g.create_texture_view(&color, None);
    let tiny = f32::from_bits(0x2b80_0000); //2^-40, normal rather than subnormal f32.
    for normal in [[0.0; 3], [tiny, 0.0, 0.0], [-tiny, 0.0, 0.0],
                   [0.0, tiny, 0.0], [0.0, -tiny, 0.0], [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]] {
        let fallback = [0.125_f32, 0.25, 0.5]; //exactly representable in both f32 and f16.
        let values = [[normal[0], normal[1], normal[2], 0.0],
                      [fallback[0], fallback[1], fallback[2], 0.0]];
        g.write_buffer(&input, 0, bytemuck::cast_slice(&values));
        let mut encoder = g.create_encoder("actual probe helper zero contract");
        g.encode_pass(&mut encoder, "actual probe helper zero contract",
            Some(&ColorAttachment { view: &view, clear: Some([0.0; 4]), resolve: None }),
            None, &[
                Command::SetPipeline(&pipeline), Command::SetBindGroup(0, &bind),
                Command::Draw { vertices: 0..3, instances: 0..1 },
            ]);
        g.submit(encoder);
        let pixels = g.read_texture(&color);
        assert_eq!(pixels.len(), 8);
        let actual = orr_render::orr_rhi::decode_rgba16f_texel(pixels.try_into().unwrap());
        if normal == [0.0; 3] {
            assert_eq!(actual, [fallback[0], fallback[1], fallback[2], 1.0],
                "actual production helper must preserve exact zero fallback");
        } else {
            near(actual, [2.0, 3.0, 4.0], "actual production helper must retain the admitted 2^-40 nonzero directions");
        }
    }
    // Constant SH0 checks the zero/nonzero gate, not directional SH sign correctness.
}
