//! Optional CPU-only, visual background tile layers. No filesystem authority,
//! simulation state, image decoder or GPU resources live in this crate.
//! Rows run down from a world-space upper-left origin; world Y runs up. Atlas
//! rectangles retain `orr_sprite`'s top-left UV convention. Tile pivots are ignored.
use orr_sprite::SpriteDocument;
use serde::{de, Deserialize, Deserializer, Serialize};
use std::{fmt, marker::PhantomData};

pub const VERSION: u32 = 1;
pub const MAX_BYTES: usize = 64 * 1024;
pub const MAX_DIMENSION: u32 = 64;
pub const MAX_CELLS: usize = 4096;
pub const MIN_UNITS_PER_PIXEL: f32 = 0.0001;
pub const MAX_UNITS_PER_PIXEL: f32 = 100.0;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SpriteRef {
    pub package: String,
    pub document: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellCoord {
    pub column: u32,
    pub row: u32,
}

/// Unvalidated authoring data. Use `validate` before rendering or editing.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Document {
    pub version: u32,
    pub sprite: SpriteRef,
    pub width: u32,
    pub height: u32,
    pub tile_size: [u32; 2],
    pub origin: [f32; 2],
    pub units_per_pixel: f32,
    pub cells: Vec<Option<u32>>,
}

// Derived struct visitors also accept positional sequences. Route them through
// deserialize_map so every record in this JSON format is strictly an object,
// while retaining serde's duplicate-field and missing-field diagnostics.
fn object<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    struct Object<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> de::Visitor<'de> for Object<T> {
        type Value = T;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a JSON object")
        }
        fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(de::value::MapAccessDeserializer::new(map))
        }
    }
    d.deserialize_map(Object(PhantomData))
}

impl<'de> Deserialize<'de> for SpriteRef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            package: String,
            document: String,
        }
        let w: Wire = object(d)?;
        Ok(Self {
            package: w.package,
            document: w.document,
        })
    }
}

fn bounded_cells<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Option<u32>>, D::Error> {
    struct Cells;
    impl<'de> de::Visitor<'de> for Cells {
        type Value = Vec<Option<u32>>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an array of at most 4096 null or u32 cells")
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut cells = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(MAX_CELLS));
            while let Some(cell) = seq.next_element::<Option<u32>>()? {
                if cells.len() == MAX_CELLS {
                    return Err(de::Error::custom("cells exceeds 4096-cell limit"));
                }
                cells.push(cell);
            }
            Ok(cells)
        }
    }
    d.deserialize_seq(Cells)
}

impl<'de> Deserialize<'de> for Document {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            version: u32,
            sprite: SpriteRef,
            width: u32,
            height: u32,
            tile_size: [u32; 2],
            origin: [f32; 2],
            units_per_pixel: f32,
            #[serde(deserialize_with = "bounded_cells")]
            cells: Vec<Option<u32>>,
        }
        let w: Wire = object(d)?;
        Ok(Self {
            version: w.version,
            sprite: w.sprite,
            width: w.width,
            height: w.height,
            tile_size: w.tile_size,
            origin: w.origin,
            units_per_pixel: w.units_per_pixel,
            cells: w.cells,
        })
    }
}

impl Document {
    /// Parse bounded JSON and check structure; atlas references still need `validate`.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("tilemap JSON exceeds 64 KiB byte limit".into());
        }
        let document: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("malformed tilemap JSON: {e}"))?;
        document.validate_structure()?;
        Ok(document)
    }

    pub fn validate_structure(&self) -> Result<(), String> {
        if self.version != VERSION {
            return Err(format!("unsupported tilemap version {}", self.version));
        }
        if self.sprite.package.trim().is_empty() {
            return Err("sprite.package must be nonempty".into());
        }
        let path = &self.sprite.document;
        if path.trim().is_empty()
            || path.contains([':', '\\'])
            || path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err("sprite.document must be a portable relative forward-slash path".into());
        }
        if !(1..=MAX_DIMENSION).contains(&self.width) || !(1..=MAX_DIMENSION).contains(&self.height)
        {
            return Err("width and height must be in 1..=64".into());
        }
        let count = self
            .width
            .checked_mul(self.height)
            .ok_or("width * height overflow")? as usize;
        if count > MAX_CELLS || self.cells.len() != count {
            return Err(format!(
                "cells length {} must equal width * height ({count}), at most 4096",
                self.cells.len()
            ));
        }
        if self
            .tile_size
            .iter()
            .any(|v| *v == 0 || *v > orr_sprite::MAX_ATLAS_DIMENSION)
        {
            return Err(
                "tile_size dimensions must be positive and within atlas dimension limit".into(),
            );
        }
        if !self.units_per_pixel.is_finite()
            || !(MIN_UNITS_PER_PIXEL..=MAX_UNITS_PER_PIXEL).contains(&self.units_per_pixel)
        {
            return Err("units_per_pixel must be finite and in 0.0001..=100".into());
        }
        if self.origin.iter().any(|v| !v.is_finite()) {
            return Err("origin must be finite".into());
        }
        let size = self.world_size();
        let extents = [
            self.origin[0] + self.width as f32 * size[0],
            self.origin[1] - self.height as f32 * size[1],
        ];
        if size.iter().chain(extents.iter()).any(|v| !v.is_finite()) {
            return Err("tilemap final world extents must be finite".into());
        }
        // Check the exact arithmetic used by placements, not a different formula.
        for row in 0..self.height {
            for column in 0..self.width {
                if self
                    .position(CellCoord { column, row })
                    .iter()
                    .any(|v| !v.is_finite())
                {
                    return Err(format!("cell ({column},{row}) world center must be finite"));
                }
            }
        }
        Ok(())
    }

    pub fn validate(&self, sprite: &SpriteDocument) -> Result<ValidatedDocument, String> {
        self.validate_structure()?;
        for (index, cell) in self.cells.iter().enumerate() {
            if let Some(id) = cell {
                let region = sprite
                    .region(*id)
                    .ok_or_else(|| format!("cell {index} references nonexistent region {id}"))?;
                if [region.width, region.height] != self.tile_size {
                    return Err(format!(
                        "cell {index} region {id} size does not match tile_size"
                    ));
                }
            }
        }
        Ok(ValidatedDocument {
            inner: self.clone(),
        })
    }

    fn world_size(&self) -> [f32; 2] {
        self.tile_size.map(|v| v as f32 * self.units_per_pixel)
    }
    fn position(&self, cell: CellCoord) -> [f32; 2] {
        let size = self.world_size();
        [
            self.origin[0] + (cell.column as f32 + 0.5) * size[0],
            self.origin[1] - (cell.row as f32 + 0.5) * size[1],
        ]
    }
    fn index(&self, cell: CellCoord) -> Result<usize, String> {
        if cell.column >= self.width || cell.row >= self.height {
            return Err(format!(
                "cell ({},{}) outside {}x{} dimensions",
                cell.column, cell.row, self.width, self.height
            ));
        }
        Ok((cell.row * self.width + cell.column) as usize)
    }
}

/// Immutable validated data; callers cannot manufacture unchecked placements.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedDocument {
    inner: Document,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub cell: CellCoord,
    pub region: u32,
    pub position: [f32; 2],
    pub size: [f32; 2],
}

impl ValidatedDocument {
    pub fn document(&self) -> &Document {
        &self.inner
    }
    pub fn occupied_count(&self) -> usize {
        self.inner
            .cells
            .iter()
            .filter(|cell| cell.is_some())
            .count()
    }
    pub fn cell(&self, cell: CellCoord) -> Result<Option<u32>, String> {
        Ok(self.inner.cells[self.inner.index(cell)?])
    }
    /// Occupied cells only, allocation-free, stable row-major order.
    pub fn placements(&self) -> impl Iterator<Item = Placement> + '_ {
        self.inner
            .cells
            .iter()
            .enumerate()
            .filter_map(|(index, region)| {
                let region = (*region)?;
                let cell = CellCoord {
                    column: index as u32 % self.inner.width,
                    row: index as u32 / self.inner.width,
                };
                Some(Placement {
                    cell,
                    region,
                    position: self.inner.position(cell),
                    size: self.inner.world_size(),
                })
            })
    }
    /// Return a validated candidate. Source is unchanged on success, no-op or failure.
    pub fn with_cell(
        &self,
        cell: CellCoord,
        region: Option<u32>,
        sprite: &SpriteDocument,
    ) -> Result<Self, String> {
        let index = self.inner.index(cell)?;
        let mut candidate = self.inner.clone();
        candidate.cells[index] = region;
        candidate.validate(sprite)
    }
}

#[cfg(test)]
mod tests;
