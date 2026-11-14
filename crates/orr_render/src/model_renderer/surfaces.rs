//! Surface shaders on the instances of a static model (see [`crate::surface`]
//! and [`crate::model_surfaces`]).

use super::{ModelRenderer, VERTEX_ATTRS};
use crate::model_surfaces::{DrawInputs, ModelSurface, ModelSurfaces, SurfaceDraw, SurfaceGpu, VertexInput};
use crate::surface::{SurfaceId, SurfaceShader, MODEL_GLUE};
use crate::surface_texture::{SurfaceTexture, TextureId};
use orr_rhi::Rhi;

/// The surface pipelines of a static model on its vertex layout.
pub(super) fn model_surfaces<B: Rhi>(engine: String) -> ModelSurfaces<B> {
    ModelSurfaces::new(engine, VertexInput { glue: MODEL_GLUE, vs_entry: "vs_model_surface", stride: 32, attrs: VERTEX_ATTRS.to_vec() })
}

impl<B: Rhi> ModelRenderer<B> {
    /// Adds pipelines for `shader` and returns the id that
    /// [`Self::set_instance_surfaces`] uses to pick it.
    /// [`SurfaceShader::compile`] validated the shader against this
    /// renderer's own shader, so this creates no invalid GPU object.
    pub fn add_surface(&mut self, shader: &SurfaceShader) -> SurfaceId {
        self.surfaces.add(&self.rhi, self.format, shader)
    }

    /// Replaces the shader of `id` with `shader`, keeping the id. An id this
    /// renderer does not have is left alone.
    pub fn replace_surface(&mut self, id: SurfaceId, shader: &SurfaceShader) {
        self.surfaces.replace(&self.rhi, self.format, id, shader);
    }

    /// Adds an image for surface shader texture slots and returns the id that
    /// [`ModelSurface::textures`] uses to pick it.
    pub fn add_texture(&mut self, texture: &SurfaceTexture) -> TextureId {
        self.surfaces.add_texture(&self.rhi, texture)
    }

    /// Replaces the image of `id`, keeping the id. An id this renderer does
    /// not have is left alone.
    pub fn replace_texture(&mut self, id: TextureId, texture: &SurfaceTexture) {
        self.surfaces.replace_texture(&self.rhi, id, texture);
    }

    /// Surface shaders of the instances that the next frames draw in an
    /// imported scene, by instance index. `None`, a missing entry and an id
    /// this renderer does not have keep the built-in look. `time` is the clock
    /// in seconds that the shaders read. Numbers that are not finite are drawn
    /// as zero.
    pub fn set_instance_surfaces(&mut self, surfaces: &[Option<ModelSurface>], time: f32) {
        self.surfaces.set_uses(surfaces, time);
    }

    /// The pipelines and bindings that draw `index` uses, or `None` for the built-in look.
    pub(super) fn active_surface(&self, index: usize) -> Option<(&SurfaceGpu<B>, &SurfaceDraw<B>)> {
        let draw = self.instance_draws.get(index)?.surface.as_ref();
        self.surfaces.active(index / self.draws.len(), draw)
    }

    /// Builds missing bindings and writes the values of the first `count` instance draws.
    pub(super) fn write_surfaces(&mut self, count: usize) {
        for index in 0..count {
            let primitive = &self.model.source().primitives[index % self.draws.len()];
            let image = &self.views[self.model.source().materials[primitive.material as usize].image as usize];
            let draw = &mut self.instance_draws[index];
            let inputs = DrawInputs { globals: &self.globals, image, object: &draw.uniform };
            self.surfaces.write(&self.rhi, index / self.draws.len(), &mut draw.surface, inputs);
        }
    }
}
