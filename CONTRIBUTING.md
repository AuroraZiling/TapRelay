# Contributing

TapRelay is a personal project. Bug fixes, documentation changes and translations can be submitted directly as pull requests. Open an issue before working on a new feature, an architectural change or support for another platform.

The maintainer decides scope and whether to merge a change. Discussion does not guarantee acceptance. There is no fixed response time.

## Issues

Search existing issues before opening one. Use the bug report or feature request form in English or Chinese. For usage questions, open a blank issue.

For bugs, include the TapRelay version, Windows version, relevant hardware and steps to reproduce. Distinguish what happened from what you expected. Review logs before attaching them and remove personal paths, device identifiers or other information you do not want to publish.

## Changes

Keep each pull request focused. Explain the problem, the resulting behavior and how you checked it. Link related issues. Include screenshots for UI changes and state which hardware checks remain untested.

- Keep domain logic in `taprelay-core`, Windows integration in `taprelay-windows` and presentation coordination in `taprelay-app`.
- Follow existing naming and formatting. Add comments for workarounds or code that is difficult to understand.
- Update both files in `crates/taprelay-app/i18n/` when changing interface text. Remove unused translation keys.

## Local checks

See the [README](README.md#build) for build requirements. Run the relevant checks from the repository root:

```powershell
cargo fmt --all -- --check
$env:SLINT_EMIT_DEBUG_INFO = '1'
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
Remove-Item Env:SLINT_EMIT_DEBUG_INFO
cargo build --workspace --locked
```

Slint's debug metadata enables element queries in the headless UI tests; it is separate from Rust/PDB debug information. Keep it enabled across Clippy and tests to reuse generated UI code. Changing it reruns UI generation. If it was already set in your shell, restore its previous value instead of removing it.

The default development profile prioritizes incremental builds and disk usage. Use `--profile debugging` for full project and dependency symbols. See [build profiles and measurements](docs/build-performance.md) for details.

## Conduct and licensing

Follow the [Code of conduct](CODE_OF_CONDUCT.md). Submit only work you have permission to contribute. Contributions to TapRelay's original code use the project's [GNU GPL version 3 only license](LICENSE) (`GPL-3.0-only`). Preserve license notices for code and assets from other projects.

Ordinary Cargo dependencies do not require a notice entry by default. When copying or vendoring code or assets, preserve upstream notices and follow the short [third-party maintenance rules](THIRD_PARTY_NOTICES.md#maintenance). Review unusual licenses separately; binary redistribution notices belong to release packaging.
