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
a `vX.Y.Z` tag is pushed. It publishes a signed, notarized installer for each
architecture, alongside its Velopack feed and full update package. The installer
supports both `/Applications` and `~/Applications`.

Add these repository secrets under **Settings → Secrets and variables → Actions**:

| Secret | Value |
| --- | --- |
| `MACOS_CERTIFICATE_BASE64` | Base64 of one `.p12` containing the existing Developer ID Application identity and a Developer ID Installer identity from the same team, both with their private keys. |
| `MACOS_CERTIFICATE_PASSWORD` | Password set when exporting the `.p12`. |
| `MACOS_NOTARY_KEY_BASE64` | Base64 of an App Store Connect team API key (`.p8`). |
| `MACOS_NOTARY_KEY_ID` | The API key's Key ID. |
| `MACOS_NOTARY_ISSUER` | The API key's Issuer ID. |

Export one identity of each type. The installer packages the existing TapRelay
bundle (`io.github.auroraziling.taprelay`); retain the application signing team
so macOS can recognize the app's previously granted permissions.

To reproduce the CI installer locally, install the workflow's pinned `vpk` tool,
put both identities and the notarytool profile in the same keychain on the user
keychain search list, and run:

```sh
TAPRELAY_NOTARY_PROFILE=taprelay scripts/package-macos.sh --release --notarize
```

The installer, `releases.osx-<arch>.json`, and its full `.nupkg` are written to
`artifacts/`. To disambiguate identities, set `TAPRELAY_SIGN_IDENTITY` for the
app or `TAPRELAY_INSTALL_IDENTITY` for the installer to an exact certificate
name or SHA-1 fingerprint. Use `TAPRELAY_SIGN_KEYCHAIN` for a non-default keychain.
