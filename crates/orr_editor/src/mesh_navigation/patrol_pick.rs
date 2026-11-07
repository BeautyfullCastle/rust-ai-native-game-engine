//! One-shot row identity for explicit patrol surface picking.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    context: Context,
    form: Form,
    generation: u64,
    row: usize,
    host_epoch: u64,
    pin: MeshScenePin,
    source: [String; 2],
}
fn capture(
    state: &State,
    context: Context,
    row: usize,
    host_epoch: u64,
    pin: MeshScenePin,
    source: [String; 2],
) -> Result<Token, String> {
    if state.stale || state.context.as_ref() != Some(&context) || state.generation == u64::MAX {
        return Err("Patrol source changed or row identity expired; reload before picking".into());
    }
    if !state.form.enabled || state.form.points.len() > MAX_POINTS || row >= state.form.points.len()
    {
        return Err("Select an existing enabled patrol waypoint".into());
    }
    Ok(Token {
        context,
        form: state.form.clone(),
        generation: state.generation,
        row,
        host_epoch,
        pin,
        source,
    })
}
fn stage(state: &mut State, current: Token, token: &Token, point: [FP; 3]) -> Result<(), String> {
    if current != *token {
        return Err("Patrol draft or source changed; pick the waypoint again".into());
    }
    if point
        .iter()
        .any(|v| v.raw().unsigned_abs() > (256 * FP::SCALE) as u64)
    {
        return Err("Patrol waypoint exceeds -256..256".into());
    }
    state.form.points[token.row] = point.map(format_fixed);
    state.generation = state.generation.saturating_add(1);
    state.clear_inspection();
    state.dirty = true;
    state.status=format!("Patrol waypoint {} captured in draft; Apply validates every leg, then save the scene explicitly.",token.row+1);
    Ok(())
}
impl Panel {
    pub(crate) fn patrol_pick_token(&self, editor: &Editor, row: usize) -> Result<Token, String> {
        let (context, view) = self.patrol_context(editor)?;
        capture(
            &self.patrol,
            context,
            row,
            editor.sim().epoch,
            view.pin,
            [self.asset_path.clone(), self.model_path.clone()],
        )
    }
    pub(crate) fn stage_patrol_pick(
        &mut self,
        editor: &Editor,
        token: &Token,
        point: [FP; 3],
    ) -> Result<(), String> {
        let (context, view) = self.patrol_context(editor)?;
        let current = capture(
            &self.patrol,
            context,
            token.row,
            editor.sim().epoch,
            view.pin,
            [self.asset_path.clone(), self.model_path.clone()],
        )?;
        if current != *token {
            return Err("Patrol source changed; arm the waypoint pick again".into());
        }
        let projected = view
            .graph
            .project(point, view.pin.max_vertical)
            .map_err(|e| e.to_string())?;
        if projected.position != point {
            return Err("Patrol pick must be on the admitted surface".into());
        }
        stage(&mut self.patrol, current, token, point)
    }
}
#[cfg(test)]
#[path = "patrol_pick_tests.rs"]
mod tests;
