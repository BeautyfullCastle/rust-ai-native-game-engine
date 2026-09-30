//! Compact, human-readable text for ERP results (what a model reads first;
//! the structured JSON goes along with it).

use serde_json::Value as J;

fn s(v: &J) -> &str {
    v.as_str().unwrap_or("")
}

/// A number as its exact text (fixed-point values are exact decimals).
fn num(v: &J) -> String {
    match v {
        J::Null => "-".to_string(),
        other => other.to_string(),
    }
}

/// The `proposal.verify` / `verify.self` report as text.
pub fn verify_text(r: &J) -> String {
    let inputs = &r["inputs"];
    let mut how = s(&inputs["kind"]).to_string();
    if let Some(t) = inputs["seed"].as_u64() {
        how.push_str(&format!(", seed {t}"));
    }
    if let Some(p) = inputs["players"].as_u64() {
        how.push_str(&format!(", {p} players"));
    }
    let subject = match r["proposal"].as_str() {
        Some(id) => format!("Proposal {id}"),
        None => "Baseline (scene alone)".to_string(),
    };
    let mut out = format!("{subject}: replayed {} ticks ({}..{}; inputs: {how}).\n", r["ticks"], r["start_tick"], r["end_tick"]);

    match r["checks"]["passed"].as_bool() {
        Some(ok) => {
            let results = r["checks"]["results"].as_array().map(Vec::as_slice).unwrap_or_default();
            let passed = results.iter().filter(|c| c["passed"] == true).count();
            out.push_str(&format!("CHECKS {}: {passed}/{} passed.\n", if ok { "PASSED" } else { "FAILED" }, results.len()));
            for c in results {
                out.push_str(&format!("  [{}] {}  ({})\n", if c["passed"] == true { "pass" } else { "FAIL" }, s(&c["check"]), s(&c["reason"])));
            }
        }
        None => out.push_str("No checks given: pass `checks` (e.g. [\"lost_bodies.max == 0\"]) to get pass/fail.\n"),
    }

    let is_baseline = r["proposal"].is_null();
    let c = &r["checksums"];
    if is_baseline {
        out.push_str(&format!("Checksum {} -> {}.\n", s(&c["base_start"]), s(&c["base_final"])));
    } else if r["identical"] == true {
        out.push_str(&format!("Checksums identical at every tick (final {}): the proposal does not change the run.\n", s(&c["base_final"])));
    } else {
        out.push_str(&format!(
            "Checksums: base {} -> {}, candidate {} -> {}; they differ from tick {} (any edit differs from tick 0: judge by metrics).\n",
            s(&c["base_start"]),
            s(&c["base_final"]),
            s(&c["candidate_start"]),
            s(&c["candidate_final"]),
            num(&r["first_divergence"])
        ));
    }
    if let Some(rec) = r["recording"].as_object() {
        out.push_str(&format!(
            "Recording: {} recorded checksums compared, {} differ from the base run.\n",
            num(&rec["checked"]),
            num(&rec["mismatches"])
        ));
    }

    let metrics = r["metrics"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut same: Vec<&str> = Vec::new();
    if is_baseline {
        out.push_str("Metrics (start -> final, min..max):\n");
        for m in metrics {
            let b = &m["base"];
            out.push_str(&format!("  {}: {} -> {}  ({}..{})\n", s(&m["name"]), num(&b["start"]), num(&b["end"]), num(&b["min"]), num(&b["max"])));
        }
    } else {
        let mut lines = Vec::new();
        for m in metrics {
            let (b, c) = (&m["base"], &m["candidate"]);
            let mut parts = Vec::new();
            for (label, key) in [("start", "start"), ("final", "end"), ("min", "min"), ("max", "max")] {
                if b[key] != c[key] {
                    parts.push(format!("{label} {} -> {}", num(&b[key]), num(&c[key])));
                }
            }
            if parts.is_empty() {
                same.push(s(&m["name"]));
            } else {
                lines.push(format!("  {}: {} (final delta {})\n", s(&m["name"]), parts.join("; "), num(&m["delta"])));
            }
        }
        if lines.is_empty() {
            out.push_str("No metric differs between base and candidate.\n");
        } else {
            out.push_str("Metrics that differ (base -> candidate):\n");
            for l in lines {
                out.push_str(&l);
            }
        }
        if !same.is_empty() {
            out.push_str(&format!("Unchanged: {}.\n", same.join(", ")));
        }
    }
    out
}

/// A `sim.state` result as one line.
pub fn state_text(r: &J) -> String {
    if r["mode"] == "play" {
        format!(
            "Play session: tick {} (recorded {}..={}), {}, {} players, checksum {}, {} entities.\n",
            r["head_tick"],
            r["first_tick"],
            r["last_tick"],
            if r["playing"] == true { "running" } else { "paused" },
            r["player_count"],
            s(&r["checksum"]),
            r["entities"]
        )
    } else {
        format!("Edit mode (no play session): scene checksum {}, {} entities.\n", s(&r["checksum"]), r["entities"])
    }
}

/// A `history.list` result as lines.
pub fn history_text(r: &J) -> String {
    let entries = r["entries"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut out = if entries.is_empty() { "History is empty.\n".to_string() } else { format!("{} history entries (oldest first):\n", entries.len()) };
    for e in entries {
        out.push_str(&format!(
            "  #{}  \"{}\"  by {}  {} op(s){}\n",
            e["id"],
            s(&e["label"]),
            s(&e["origin"]),
            e["op_count"],
            if e["undone"] == true { "  (undone)" } else { "" }
        ));
    }
    out.push_str(&format!("can_undo: {}, can_redo: {}, unsaved changes: {}.\n", r["can_undo"], r["can_redo"], r["dirty"]));
    out
}
