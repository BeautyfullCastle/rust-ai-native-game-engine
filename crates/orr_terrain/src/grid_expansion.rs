//! Bounded +X/+Z grid growth, separate from the per-sample edit batch budget.
use crate::{Error, Terrain, TerrainDocument, MAX_SIDE};
use orr_fp::FP;

impl Terrain {
    /// Admit dimensions and far-edge coordinates without allocation or mutation.
    /// Equal dimensions are valid; either axis shrinking is rejected.
    pub fn validate_expansion(&self, width: u32, depth: u32) -> Result<(), Error> {
        if !(2..=MAX_SIDE).contains(&width) || !(2..=MAX_SIDE).contains(&depth) {
            return Err(Error("terrain dimension limit"));
        }
        if width < self.width || depth < self.depth {
            return Err(Error(
                "terrain expansion cannot shrink either grid dimension",
            ));
        }
        for (origin, side) in self.origin.iter().zip([width, depth]) {
            let end =
                i128::from(origin.raw()) + i128::from(side - 1) * i128::from(self.spacing.raw());
            if i64::try_from(end).is_err() {
                return Err(Error("terrain coordinate overflow"));
            }
        }
        Ok(())
    }

    /// Return a larger immutable grid, preserving its identity and world anchor.
    /// Old vertex/cell coordinates retain their exact values at the new row strides.
    /// Added vertices use `fill`; added cells are solid. Equal dimensions are a no-op.
    pub fn expanded_grid(&self, width: u32, depth: u32, fill: FP) -> Result<Self, Error> {
        self.validate_expansion(width, depth)?;
        if width == self.width && depth == self.depth {
            return Ok(self.clone());
        }
        let mut heights = vec![fill; (width * depth) as usize];
        for z in 0..self.depth {
            let before = (z * self.width) as usize;
            let after = (z * width) as usize;
            heights[after..after + self.width as usize]
                .copy_from_slice(&self.heights[before..before + self.width as usize]);
        }
        let mut holes = vec![false; ((width - 1) * (depth - 1)) as usize];
        for z in 0..self.depth - 1 {
            let before = (z * (self.width - 1)) as usize;
            let after = (z * (width - 1)) as usize;
            holes[after..after + (self.width - 1) as usize]
                .copy_from_slice(&self.holes[before..before + (self.width - 1) as usize]);
        }
        Self::new(
            self.asset_id.clone(),
            width,
            depth,
            self.origin,
            self.spacing,
            heights,
            holes,
        )
    }
}

impl TerrainDocument {
    /// Grow at the +X/+Z edges as one bounded undo entry. No-op or rejected growth
    /// preserves history, redo and the most recent dirty region.
    /// Successful growth dirties every cell in the resulting grid.
    pub fn expand_grid(&mut self, width: u32, depth: u32, fill: FP) -> Result<(), Error> {
        let candidate = self.terrain.expanded_grid(width, depth, fill)?;
        self.commit_candidate(candidate);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
