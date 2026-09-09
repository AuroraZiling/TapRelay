//! Synthetic CPU benchmark, not an end-to-end BLE or GUI latency measurement.
//! Run with `cargo run --release -p taprelay-core --example matching_bench`.
use std::{hint::black_box, time::Instant};
use taprelay_core::{
    binding::{self, Binding, BindingIndex},
    command::MediaCommand,
    input::{InputCode, InputEvent, InputState, Trigger},
};

fn scan(bindings: &[Binding], code: InputCode, state: &InputState) -> Option<usize> {
    bindings
        .iter()
        .enumerate()
        .filter(|(_, b)| b.enabled && b.tap.contains(code) && state.matches(&b.tap))
        .max_by_key(|(_, b)| b.tap.specificity())
        .map(|(i, _)| i)
}
fn old_validation(bindings: &[Binding]) -> bool {
    bindings
        .iter()
        .enumerate()
        .all(|(i, b)| b.tap.valid() && !bindings[..i].iter().any(|p| p.tap == b.tap))
}
fn timed(iterations: u32, mut work: impl FnMut()) -> f64 {
    let start = Instant::now();
    for _ in 0..iterations {
        work();
    }
    start.elapsed().as_nanos() as f64 / f64::from(iterations)
}
fn main() {
    println!("bindings,scan_ns,indexed_ns,old_validation_ns,set_validation_ns");
    for count in [10, 100, 1000, 5000] {
        let bindings: Vec<_> = (8u8..=200)
            .flat_map(|a| (a + 1..=254).map(move |b| (a, b)))
            .map(|(a, b)| Binding {
                tap: Trigger::Keyboard { keys: vec![a, b] },
                relay: MediaCommand::PlayPause,
                enabled: true,
            })
            .filter(|b| b.tap.valid())
            .take(count)
            .collect();
        let index = BindingIndex::new(&bindings);
        let mut state = InputState::default();
        for key in [8, 0x41] {
            state.update(InputEvent {
                code: InputCode::Key(key),
                down: true,
                captured: Instant::now(),
            });
        }
        let code = InputCode::Key(0x41);
        assert_eq!(
            scan(&bindings, code, &state),
            index.best_match(code, &state)
        );
        assert_eq!(old_validation(&bindings), binding::valid(&bindings));
        let scan_ns = timed(20_000, || {
            black_box(scan(
                black_box(&bindings),
                black_box(code),
                black_box(&state),
            ));
        });
        let indexed_ns = timed(20_000, || {
            black_box(index.best_match(black_box(code), black_box(&state)));
        });
        let old_ns = timed(20, || {
            black_box(old_validation(black_box(&bindings)));
        });
        let set_ns = timed(20, || {
            black_box(binding::valid(black_box(&bindings)));
        });
        println!("{count},{scan_ns:.1},{indexed_ns:.1},{old_ns:.1},{set_ns:.1}");
    }
}
