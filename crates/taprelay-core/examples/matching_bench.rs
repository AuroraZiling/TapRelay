//! Synthetic CPU benchmark, not an end-to-end BLE or GUI latency measurement.
//! Run with `cargo run --release -p taprelay-core --example matching_bench`.
use std::{hint::black_box, time::Instant};
use taprelay_core::{
    binding::{self, BindingIndex},
    function::{FunctionConfig, FunctionId, ModifierSet, Shortcut, default_configs},
    input::{InputCode, InputEvent, InputState},
};

fn scan(
    configs: &taprelay_core::function::FunctionConfigs,
    code: InputCode,
    state: &InputState,
) -> Option<usize> {
    BindingIndex::new(configs).best_match(code, state)
}

fn old_validation(configs: &taprelay_core::function::FunctionConfigs) -> bool {
    let mut values: Vec<&taprelay_core::function::Shortcut> = Vec::new();
    for config in configs.values() {
        for shortcut in &config.shortcuts {
            if !shortcut.valid() || values.contains(&shortcut) {
                return false;
            }
            values.push(shortcut);
        }
    }
    true
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
        let mut configs = default_configs();
        let mut shortcuts = Vec::new();
        for key in 8u8..=254 {
            shortcuts.push(Shortcut::keyboard(ModifierSet::empty(), key));
            if shortcuts.len() == count {
                break;
            }
        }
        configs.insert(
            FunctionId::MediaNext,
            FunctionConfig {
                enabled: true,
                shortcuts: shortcuts.into_iter().take(2).collect(),
            },
        );
        let index = BindingIndex::new(&configs);
        let mut state = InputState::default();
        state.update(InputEvent {
            code: InputCode::Key(0x41),
            down: true,
            captured: Instant::now(),
        });
        let code = InputCode::Key(0x41);
        assert_eq!(scan(&configs, code, &state), index.best_match(code, &state));
        assert_eq!(old_validation(&configs), binding::valid(&configs));
        let scan_ns = timed(20_000, || {
            black_box(scan(
                black_box(&configs),
                black_box(code),
                black_box(&state),
            ));
        });
        let indexed_ns = timed(20_000, || {
            black_box(index.best_match(black_box(code), black_box(&state)));
        });
        let old_ns = timed(20, || {
            black_box(old_validation(black_box(&configs)));
        });
        let set_ns = timed(20, || {
            black_box(binding::valid(black_box(&configs)));
        });
        println!("{count},{scan_ns:.1},{indexed_ns:.1},{old_ns:.1},{set_ns:.1}");
    }
}
