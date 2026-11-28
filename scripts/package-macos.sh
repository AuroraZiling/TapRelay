#!/bin/bash
# Development and Developer ID distribution share this exact bundle layout.
set -euo pipefail
cd "$(dirname "$0")/.."
mode=development
profile=release
build=true
notarize=false
target=
while (($#)); do
    case "$1" in
        --release) mode=distribution ;;
        --debug) profile=debug ;;
        --skip-build) build=false ;;
        --notarize) notarize=true ;;
        --target)
            [[ $# -ge 2 && -n $2 ]] || { echo "--target requires a Rust target triple" >&2; exit 2; }
            target=$2
            shift
            ;;
        *) echo "Usage: $0 [--target aarch64-apple-darwin|x86_64-apple-darwin] [--debug] [--skip-build] [--release --notarize]" >&2; exit 2 ;;
    esac
    shift
done
[[ $(uname -s) == Darwin ]] || { echo "macOS build host required" >&2; exit 1; }
explicit_target=false
if [[ -n $target ]]; then
    explicit_target=true
else
    case $(uname -m) in
        arm64) target=aarch64-apple-darwin ;;
        x86_64) target=x86_64-apple-darwin ;;
        *) echo "Unsupported macOS host architecture" >&2; exit 1 ;;
    esac
fi
case $target in
    aarch64-apple-darwin) asset_arch=arm64; binary_arch=arm64 ;;
    x86_64-apple-darwin) asset_arch=x64; binary_arch=x86_64 ;;
    *) echo "Unsupported macOS target: $target" >&2; exit 2 ;;
esac
# Keep the binary and bundle aligned with the SMAppService baseline.
export MACOSX_DEPLOYMENT_TARGET=13.0
version=${TAPRELAY_VERSION:-$(sed -n '/\[workspace.package\]/,/^\[/s/^version = "\([^"]*\)"/\1/p' Cargo.toml)}
[[ $version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || { echo "Version must use X.Y.Z" >&2; exit 1; }
if [[ $mode == distribution ]]; then
    [[ $profile == release && $notarize == true ]] || { echo "Distribution requires a release build and --notarize" >&2; exit 1; }
    : "${TAPRELAY_NOTARY_PROFILE:?Set a notarytool Keychain profile}"
    keychain=${TAPRELAY_SIGN_KEYCHAIN:-}
    if [[ -n ${TAPRELAY_NOTARY_KEYCHAIN:-} && -n $keychain && $TAPRELAY_NOTARY_KEYCHAIN != "$keychain" ]]; then
        echo "Velopack requires the signing identities and notary profile in the same keychain" >&2; exit 1
    fi
    keychain=${keychain:-${TAPRELAY_NOTARY_KEYCHAIN:-}}
    identity_args=(-v)
    [[ -z $keychain ]] || identity_args+=("$keychain")
    TAPRELAY_SIGN_IDENTITY=$(security find-identity -p codesigning "${identity_args[@]}" | /usr/bin/python3 scripts/macos-signing-identity.py "${TAPRELAY_SIGN_IDENTITY:-}")
    TAPRELAY_INSTALL_IDENTITY=$(security find-identity "${identity_args[@]}" | /usr/bin/python3 scripts/macos-signing-identity.py --installer "${TAPRELAY_INSTALL_IDENTITY:-}")
fi
if $build; then
    args=(build --locked -p taprelay-app)
    if $explicit_target || [[ $profile == release ]]; then args+=(--target "$target"); fi
    [[ $profile != release ]] || args+=(--release)
    cargo "${args[@]}"
fi
binary="target/$profile/taprelay"
if $explicit_target || [[ $profile == release ]]; then binary="target/$target/$profile/taprelay"; fi
[[ -f $binary ]] || { echo "Missing $binary" >&2; exit 1; }
actual_arch=$(xcrun lipo -archs "$binary")
[[ $actual_arch == "$binary_arch" ]] || {
    echo "Expected $binary_arch binary for $target (found $actual_arch)" >&2
    exit 1
}
binary_minimum=$(xcrun vtool -show-build "$binary" | awk '$1 == "minos" {print $2}')
[[ $binary_minimum == "$MACOSX_DEPLOYMENT_TARGET" ]] || {
    echo "Rebuild $binary with MACOSX_DEPLOYMENT_TARGET=$MACOSX_DEPLOYMENT_TARGET (found $binary_minimum)" >&2
    exit 1
}
mkdir -p artifacts
staging=$(mktemp -d "$PWD/artifacts/macos-staging.XXXXXX")
trap 'rm -rf "$staging"' EXIT
app="$staging/TapRelay.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$staging/TapRelay.iconset"
cp "$binary" "$app/Contents/MacOS/taprelay"
cp LICENSE THIRD_PARTY_NOTICES.md "$app/Contents/Resources/"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" crates/taprelay-app/resources/taprelay-icon.png --out "$staging/TapRelay.iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size*2))
    sips -z "$double" "$double" crates/taprelay-app/resources/taprelay-icon.png --out "$staging/TapRelay.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$staging/TapRelay.iconset" -o "$app/Contents/Resources/TapRelay.icns"
/usr/bin/python3 - "$app/Contents/Info.plist" "$version" "$MACOSX_DEPLOYMENT_TARGET" <<'PY'
import plistlib,sys
with open(sys.argv[1], 'wb') as f:
    plistlib.dump({
      'CFBundleIdentifier':'io.github.auroraziling.taprelay',
      'CFBundleName':'TapRelay','CFBundleDisplayName':'TapRelay',
      'CFBundleExecutable':'taprelay','CFBundlePackageType':'APPL',
      'CFBundleShortVersionString':sys.argv[2], 'CFBundleVersion':sys.argv[2],
      'CFBundleIconFile':'TapRelay.icns','LSMinimumSystemVersion':sys.argv[3],
      'NSHighResolutionCapable':True,'NSSupportsAutomaticGraphicsSwitching':True,
      'NSBluetoothAlwaysUsageDescription':'TapRelay sends media, keyboard and mouse input to your selected receiver over Bluetooth.',
      'NSBluetoothPeripheralUsageDescription':'TapRelay publishes a Bluetooth HID service for your selected receiver.',
      'LSApplicationCategoryType':'public.app-category.utilities',
      'NSPrincipalClass':'NSApplication',
      'NSHumanReadableCopyright':'TapRelay contributors. GNU GPL version 3 only.'
    }, f)
PY
plutil -lint "$app/Contents/Info.plist"
if [[ $mode == development ]]; then
    # A certificate keeps the designated requirement stable across rebuilds,
    # so TCC can recognize the same development application after an update.
    # Ad-hoc signing remains available when no identity has been configured.
    dev_sign_args=(--force --sign "${TAPRELAY_SIGN_IDENTITY:--}" --timestamp=none)
    [[ -z ${TAPRELAY_SIGN_KEYCHAIN:-} ]] || dev_sign_args+=(--keychain "$TAPRELAY_SIGN_KEYCHAIN")
    codesign "${dev_sign_args[@]}" "$app"
    codesign --verify --strict --deep --verbose=2 "$app"
    destination="$PWD/artifacts/macos-development"
    # Explicit targets keep cross-built packages separate from the running app.
    if $explicit_target; then destination="$destination-$asset_arch"; fi
    mkdir -p "$destination"
    # Replace only the development bundle owned by this script.
    rm -rf "$destination/TapRelay.app"
    mv "$app" "$destination/TapRelay.app"
    echo "$destination/TapRelay.app"
else
    channel="osx-$asset_arch"
    output="$staging/releases"
    # vpk's default entitlements relax hardened runtime for .NET/JIT, which TapRelay does not use.
    plutil -create xml1 "$staging/TapRelay.entitlements"
    args=(pack --packId TapRelay --packVersion "$version" --packAuthors "Artemis Li" --packDir "$app" --mainExe taprelay
        --channel "$channel" --runtime "osx${MACOSX_DEPLOYMENT_TARGET}-$asset_arch" --outputDir "$output" --delta None --noPortable
        --signAppIdentity "$TAPRELAY_SIGN_IDENTITY" --signInstallIdentity "$TAPRELAY_INSTALL_IDENTITY"
        --signEntitlements "$staging/TapRelay.entitlements" --notaryProfile "$TAPRELAY_NOTARY_PROFILE" --skip-updates)
    if [[ -n $keychain ]]; then args+=(--keychain "$keychain"); fi
    vpk "${args[@]}"

    pkg="$output/TapRelay-$channel-Setup.pkg"
    pkgutil --check-signature "$pkg"
    spctl --assess --type install --verbose=2 "$pkg"
    xcrun stapler validate "$pkg"
    pkgutil --expand-full "$pkg" "$staging/expanded"
    packaged_app=$(find "$staging/expanded" -type d -name 'TapRelay.app' -prune)
    codesign --verify --strict --deep --verbose=2 "$packaged_app"
    team=$(codesign -dv --verbose=4 "$packaged_app" 2>&1 | sed -n 's/^TeamIdentifier=//p')
    [[ -n $team && $TAPRELAY_INSTALL_IDENTITY == *" ($team)" ]] || { echo "App and installer must be signed by the same Developer ID team" >&2; exit 1; }
    spctl --assess --type execute --verbose=2 "$packaged_app"
    xcrun stapler validate "$packaged_app"

    name="TapRelay_${version}_macOS_${asset_arch}.pkg"
    mv "$pkg" "$output/$name"
    mv "$output/$name" "$output/releases.$channel.json" "$output/TapRelay-$version-$channel-full.nupkg" artifacts/
    echo "$PWD/artifacts/$name"
fi
