use orr_navigation::mesh::StaticMeshGraph;

const MAX_HISTORY_STATES: usize = 16;
const MAX_GRAPH_BYTES: usize = 8_192;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct History {
    states: Vec<Option<StaticMeshGraph>>,
    state_bytes: Vec<Option<Vec<u8>>>,
    cursor: usize,
    origin_bytes: Option<Option<Vec<u8>>>,
}

impl History {
    pub(crate) fn reset(&mut self, origin: Option<&StaticMeshGraph>) -> Result<(), String> {
        let serialized = serialize_graph(origin)?;
        let state = origin.cloned();
        self.states.clear();
        self.state_bytes.clear();
        self.states.push(state);
        self.state_bytes.push(serialized.clone());
        self.cursor = 0;
        self.origin_bytes = Some(serialized);
        Ok(())
    }

    pub(crate) fn push(&mut self, graph: Option<&StaticMeshGraph>) -> Result<bool, String> {
        if self.origin_bytes.is_none() {
            return Err("geometry history has not been initialized".into());
        }
        let serialized = serialize_graph(graph)?;
        if self.state_bytes.get(self.cursor) == Some(&serialized) {
            return Ok(false);
        }

        let next_cursor = self
            .cursor
            .checked_add(1)
            .ok_or_else(|| "geometry history cursor overflow".to_owned())?;
        let next_graph = graph.cloned();
        self.states.truncate(next_cursor);
        self.state_bytes.truncate(next_cursor);
        self.states.push(next_graph);
        self.state_bytes.push(serialized);
        self.cursor = next_cursor;

        if self.states.len() > MAX_HISTORY_STATES {
            self.states.remove(0);
            self.state_bytes.remove(0);
            self.cursor = next_cursor - 1;
        }
        Ok(true)
    }

    pub(crate) fn undo(&mut self) -> Option<Option<StaticMeshGraph>> {
        if self.cursor == 0 || self.origin_bytes.is_none() {
            return None;
        }
        self.cursor -= 1;
        Some(self.states[self.cursor].clone())
    }

    pub(crate) fn redo(&mut self) -> Option<Option<StaticMeshGraph>> {
        if self.origin_bytes.is_none() || !self.can_redo() {
            return None;
        }
        self.cursor += 1;
        Some(self.states[self.cursor].clone())
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.origin_bytes.is_some() && self.cursor > 0
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.origin_bytes.is_some() && self.cursor < self.states.len().saturating_sub(1)
    }

    pub(crate) fn at_origin(&self) -> bool {
        self.origin_bytes
            .as_ref()
            .is_some_and(|origin| self.state_bytes.get(self.cursor) == Some(origin))
    }

    pub(crate) fn current(&self) -> Option<Option<&StaticMeshGraph>> {
        self.origin_bytes.as_ref()?;
        self.states.get(self.cursor).map(Option::as_ref)
    }
}

fn serialize_graph(graph: Option<&StaticMeshGraph>) -> Result<Option<Vec<u8>>, String> {
    let Some(graph) = graph else {
        return Ok(None);
    };
    let bytes = graph.to_bytes();
    if bytes.len() > MAX_GRAPH_BYTES {
        return Err("mesh graph exceeds the 8 KiB local history state limit".into());
    }
    let decoded = StaticMeshGraph::from_bytes(&bytes).map_err(|error| error.to_string())?;
    if decoded != *graph || decoded.to_bytes() != bytes {
        return Err("mesh graph failed canonical serialization verification".into());
    }
    Ok(Some(bytes))
}

#[path = "geometry_history/tests.rs"]
#[cfg(test)]
mod tests;
