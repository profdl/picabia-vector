//! The result store: freshness by the cheap snapshot, re-keying by geometry, eviction of the
//! oldest results and lookups renewing what is drawn.

use std::sync::{Arc, Mutex};

use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::kurbo::Rect;
use vectorcraft_geom::shapes;

use crate::store::{self, Inputs, MAX_ENTRIES, Sheet};

/// The store is process-wide: these tests take turns (no other test of this crate uses it).
static TURN: Mutex<()> = Mutex::new(());

/// A stencil object (a square with a stencil row), as the document holds it.
fn object(id: u64, x: f64) -> Arc<Node> {
    let mut n = Node::path(NodeId(id), shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::default());
    vectorcraft_modifiers::set_node_source(&mut n, Some(r#"{"v":1,"stack":[{"id":"s1","enabled":true,"kind":"stencil"}]}"#.into()));
    Arc::new(n)
}

fn sheet(tag: f64) -> Sheet {
    Sheet { sheet: Rect::new(0.0, 0.0, tag, tag), ..Sheet::default() }
}

#[test]
fn a_result_is_fresh_for_the_object_it_was_computed_from() {
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let doc = Document::new(100.0, 100.0);
    let a = object(9_100_001, 0.0);
    assert!(store::is_stencil(&a));
    assert!(store::lookup(&doc, &a).is_none(), "nothing computed yet");
    let kept = store::store(a.id, Inputs::take(&doc, &a), 7, sheet(1.0));
    let (got, fresh) = store::lookup(&doc, &a).unwrap();
    assert!(fresh && Arc::ptr_eq(&got, &kept));
    // The same object changed (a new node): stale, the latest result still drawn.
    let moved = object(9_100_001, 5.0);
    let (got, fresh) = store::lookup(&doc, &moved).unwrap();
    assert!(!fresh && Arc::ptr_eq(&got, &kept));
    assert!(!store::is_fresh(&doc, &moved));
    // Even an equal copy is another node: the snapshot compares pointers, never contents.
    let copy = Arc::new((*a).clone());
    assert!(!store::lookup(&doc, &copy).unwrap().1);
}

#[test]
fn rekey_moves_a_kept_result_to_new_inputs_with_the_same_geometry() {
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let doc = Document::new(100.0, 100.0);
    let a = object(9_200_001, 0.0);
    let generation = store::generation();
    let kept = store::store(a.id, Inputs::take(&doc, &a), 42, sheet(2.0));
    assert!(store::generation() > generation);
    let b = object(9_200_001, 0.0);
    assert!(store::rekey(a.id, 43, Inputs::take(&doc, &b)).is_none(), "another geometry: compute");
    assert!(store::rekey(NodeId(9_200_002), 42, Inputs::take(&doc, &b)).is_none(), "another object");
    let generation = store::generation();
    let again = store::rekey(a.id, 42, Inputs::take(&doc, &b)).unwrap();
    assert!(Arc::ptr_eq(&again, &kept));
    assert!(store::generation() > generation, "a re-key repaints");
    assert!(store::lookup(&doc, &b).unwrap().1, "fresh for the new inputs");
    assert!(!store::lookup(&doc, &a).unwrap().1, "and no longer for the old ones");
    // Storing the same key again replaces the result (one entry per object and key).
    let newer = store::store(a.id, Inputs::take(&doc, &a), 42, sheet(3.0));
    assert!(Arc::ptr_eq(&store::lookup(&doc, &a).unwrap().0, &newer));
    assert!(!store::lookup(&doc, &b).unwrap().1);
}

#[test]
fn the_oldest_results_go_and_a_lookup_renews_one() {
    let _turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let doc = Document::new(100.0, 100.0);
    let base = 9_300_000u64;
    let drawn = object(base, 0.0);
    store::store(drawn.id, Inputs::take(&doc, &drawn), 1, sheet(1.0));
    let first_other = object(base + 1, 0.0);
    store::store(first_other.id, Inputs::take(&doc, &first_other), 1, sheet(1.0));
    // Another object being edited fills the store; the drawn one is looked up meanwhile.
    for k in 0..MAX_ENTRIES as u64 {
        let n = object(base + 100, k as f64);
        store::store(n.id, Inputs::take(&doc, &n), k + 10, sheet(1.0));
        if k % 16 == 0 {
            assert!(store::lookup(&doc, &drawn).unwrap().1);
        }
    }
    assert!(store::lookup(&doc, &drawn).is_some_and(|(_, f)| f), "renewed by its lookups, still kept");
    assert!(store::lookup(&doc, &first_other).is_none(), "the oldest result went");
    // The store never holds more than its cap: the earliest of the edited object's results are gone.
    let earliest = object(base + 100, 0.0);
    assert!(store::lookup(&doc, &earliest).is_some_and(|(_, f)| !f), "a later result of that object still draws");
}
