# Build on macOS

Requires macOS 13 or later, rustup, and Xcode Command Line Tools.
The Rust toolchain is pinned in `rust-toolchain.toml`.

## Local build

From the repository root:

```sh
scripts/package-macos.sh --debug
open artifacts/macos-development/TapRelay.app
```

The script builds, signs, and packages the app for your Mac. Omit `--debug` for
an optimized build, or add `--skip-build` to reuse an existing binary.
On first launch, allow Bluetooth, Accessibility, and Input Monitoring when prompted.

Development builds use ad-hoc signing by default. Set `TAPRELAY_SIGN_IDENTITY`
to a signing certificate to keep the app's permission identity stable across rebuilds.

## Target architecture

To build for Intel on Apple Silicon:

```sh
rustup target add x86_64-apple-darwin
scripts/package-macos.sh --target x86_64-apple-darwin
```

The app is saved to `artifacts/macos-development-x64/TapRelay.app`.
For Apple Silicon, use `aarch64-apple-darwin`; the output directory ends in `-arm64`.

## GitHub Actions release

The [Desktop Release workflow](../.github/workflows/desktop-release.yml) runs when
a `vX.Y.Z` tag is pushed. It builds, signs, notarizes, and packages arm64 and x64
apps, then publishes their ZIPs alongside Windows after all jobs pass.

Add these repository secrets under **Settings → Secrets and variables → Actions**:

| Secret | Value |
| --- | --- |
| `MACOS_CERTIFICATE_BASE64` | Base64 of the Developer ID Application certificate and private key exported as `.p12`. |
| `MACOS_CERTIFICATE_PASSWORD` | Password set when exporting the `.p12`. |
| `MACOS_SIGN_IDENTITY` | Full certificate name: `Developer ID Application: YOUR NAME (TEAMID)`. |
| `MACOS_NOTARY_KEY_BASE64` | Base64 of an App Store Connect team API key (`.p8`). |
| `MACOS_NOTARY_KEY_ID` | The API key's Key ID. |
| `MACOS_NOTARY_ISSUER` | The API key's Issuer ID. |
