//! Several editor windows save their layout into the same folder at the same time.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

use orr_editor::layout::{Edge, Layout, LayoutStore, Panel};

const WRITERS: usize = 4;
const SAVES: usize = 60;

/// A layout that differs in size and shape for each writer.
fn layout_for(writer: usize) -> Layout {
    let mut layout = Layout::default();
    if writer % 2 == 1 {
        layout.dock(Panel::Inspector, Edge::Bottom);
        layout.set_visible(Panel::Timeline, false);
    }
    if writer >= 2 {
        layout.set_visible(Panel::Inspector, false);
    }
    layout
}

#[test]
fn concurrent_saves_keep_one_valid_file_and_leave_no_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("layouts");
    let layouts: Vec<Layout> = (0..WRITERS).map(layout_for).collect();
    let barrier = Arc::new(Barrier::new(WRITERS + 1));
    let done = Arc::new(AtomicBool::new(false));

    // Another window that reads the last layout while the others replace it.
    let reader = {
        let (store, barrier, done) = (LayoutStore::at(&folder), barrier.clone(), done.clone());
        std::thread::spawn(move || {
            barrier.wait();
            while !done.load(Ordering::Relaxed) {
                let _ = store.read_last();
            }
        })
    };
    let writers: Vec<_> = layouts
        .iter()
        .cloned()
        .map(|layout| {
            let (store, barrier) = (LayoutStore::at(&folder), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                (0..SAVES).map(|_| store.write_last(&layout)).filter_map(Result::err).map(|error| error.to_string()).collect::<Vec<_>>()
            })
        })
        .collect();
    let failures: Vec<String> = writers.into_iter().flat_map(|writer| writer.join().unwrap()).collect();
    done.store(true, Ordering::Relaxed);
    reader.join().unwrap();

    assert!(failures.is_empty(), "{} of {} saves failed, for example: {}", failures.len(), WRITERS * SAVES, failures[0]);
    let last = LayoutStore::at(&folder).read_last().expect("the last layout is valid").expect("the last layout exists");
    assert!(layouts.contains(&last), "the file holds the whole layout of one writer");
    let files: Vec<String> = std::fs::read_dir(&folder).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(files, ["last.json"], "no temporary file is left");
}
