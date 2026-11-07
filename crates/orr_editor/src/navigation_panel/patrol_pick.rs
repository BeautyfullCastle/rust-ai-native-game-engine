//! A one-shot row identity: captured drafts never retarget a different waypoint.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    context: Context,
    form: Form,
    row: usize,
    generation: u64,
}
fn capture(state: &State, context: Context, row: usize) -> Result<Token, String> {
    if state.pick_generation == u64::MAX || state.stale || state.context.as_ref() != Some(&context)
    {
        return Err("Terrain patrol source changed; reload fields before picking".into());
    }
    if !state.form.enabled || state.form.points.len() > MAX_POINTS || row >= state.form.points.len()
    {
        return Err("Select an existing enabled terrain patrol waypoint".into());
    }
    Ok(Token {
        context,
        form: state.form.clone(),
        row,
        generation: state.pick_generation,
    })
}
fn stage(state: &mut State, current: Context, token: &Token, point: [FP; 2]) -> Result<(), String> {
    if capture(state, current, token.row)? != *token {
        return Err("Terrain patrol draft changed; arm the waypoint pick again".into());
    }
    state.form.points[token.row] = point.map(format_fixed);
    state.pick_generation = state.pick_generation.saturating_add(1);
    state.dirty = true;
    state.status = format!("Terrain patrol waypoint {} captured in draft; Apply validates all route legs, then save the scene explicitly.",token.row+1);
    Ok(())
}
impl NavigationPanel {
    pub(crate) fn patrol_pick_token(
        &self,
        editor: &Editor,
        terrain: &TerrainPanel,
        row: usize,
    ) -> Result<Token, String> {
        let (context, _) = self.patrol_context(editor, terrain)?;
        capture(&self.patrol, context, row)
    }
    pub(crate) fn stage_patrol_pick(
        &mut self,
        editor: &Editor,
        terrain: &TerrainPanel,
        token: &Token,
        point: [FP; 2],
    ) -> Result<(), String> {
        let (context, view) = self.patrol_context(editor, terrain)?;
        // The picker snaps a cell center; never silently accept nearest-surface clamping.
        let projected = view
            .graph
            .project(&view.terrain, point)
            .map_err(|e| e.to_string())?;
        if projected.position[0] != point[0] || projected.position[2] != point[1] {
            return Err("Terrain patrol waypoint must lie on the selected walkable cell".into());
        }
        stage(&mut self.patrol, context, token, point)
    }
}
#[cfg(test)]
#[path = "patrol_pick_tests.rs"]
mod tests;
