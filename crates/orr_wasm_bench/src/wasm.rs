//! Exports for the browser bench page (`web/index.html`): the page times with `performance.now()`.

use std::cell::RefCell;

use wasm_bindgen::prelude::*;

use crate::cases;

thread_local! {
    static CURRENT: RefCell<Option<Box<dyn FnMut(u64) -> u64>>> = const { RefCell::new(None) };
}

#[wasm_bindgen]
pub fn case_names() -> Vec<String> {
    cases().iter().map(|c| c.name.to_string()).collect()
}

/// Iterations that make one unit for case `i` (for ns/op reports).
#[wasm_bindgen]
pub fn case_units(i: usize) -> f64 {
    cases()[i].units_per_iter as f64
}

/// Builds the state of case `i` (not timed).
#[wasm_bindgen]
pub fn prepare(i: usize) {
    let run = (cases()[i].make)();
    CURRENT.with(|c| *c.borrow_mut() = Some(run));
}

/// Runs `iters` iterations of the prepared case and returns the checksum as hex.
#[wasm_bindgen]
pub fn run(iters: f64) -> String {
    let sum = CURRENT.with(|c| (c.borrow_mut().as_mut().expect("prepare first"))(iters as u64));
    format!("{sum:016x}")
}
