mod notes;
mod package;

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result, ensure};
use semver::Version;
use velopack::{HttpOptions, VelopackAsset, VelopackAssetFeed};

pub(crate) use notes::{NoteKind, Notes, notes};
pub(crate) use package::{PreparedUpdate, download};

const RELEASES: &str = "https://github.com/AuroraZiling/TapRelay/releases";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Delivery {
    Installed,
    Portable,
}

#[derive(Clone, Debug)]
pub(crate) struct Target {
    asset: Arc<VelopackAsset>,
    delivery: Delivery,
}

impl Target {
    pub(crate) fn version(&self) -> &str {
        &self.asset.Version
    }

    pub(crate) fn tag(&self) -> String {
        format!("v{}", self.version())
    }

    pub(crate) fn release_url(&self) -> String {
        format!("{RELEASES}/tag/{}", self.tag())
    }

    pub(crate) fn portable_url(&self) -> Option<String> {
        (self.delivery == Delivery::Portable).then(|| {
            format!(
                "{RELEASES}/download/{}/TapRelay_{}_Windows_x64.zip",
                self.tag(),
                self.version()
            )
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(version: &str, portable: bool) -> Self {
        Self {
            asset: Arc::new(VelopackAsset {
                Version: version.to_owned(),
                ..Default::default()
            }),
            delivery: if portable {
                Delivery::Portable
            } else {
                Delivery::Installed
            },
        }
    }
}

#[derive(Debug)]
pub(crate) enum Check {
    Unsupported,
    Current,
    Available(Target),
}

fn stable_version(input: &str) -> Result<Version> {
    let version = Version::parse(input)?;
    ensure!(
        version.pre.is_empty() && version.build.is_empty(),
        "Expected a stable X.Y.Z version."
    );
    Ok(version)
}

pub(crate) fn is_stable_tag(input: &str) -> bool {
    input
        .strip_prefix('v')
        .is_some_and(|v| stable_version(v).is_ok())
}

fn platform_channel() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("win"),
        ("macos", "aarch64") => Some("osx-arm64"),
        ("macos", "x86_64") => Some("osx-x64"),
        _ => None,
    }
}

fn delivery(executable: &Path, channel: &str) -> Result<Option<Delivery>> {
    #[cfg(windows)]
    let manifest = match crate::installation::installed_manifest(executable)? {
        Some(manifest) => manifest,
        None => return Ok(Some(Delivery::Portable)),
    };
    #[cfg(target_os = "macos")]
    let manifest = match velopack::locator::auto_locate_app_manifest(
        velopack::locator::LocationContext::FromSpecifiedAppExecutable(executable.to_path_buf()),
    ) {
        Ok(locator) => locator.get_manifest(),
        Err(velopack::Error::NotInstalled(_)) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok((manifest.id == "TapRelay"
        && manifest.channel == channel
        && manifest.version.to_string() == crate::constants::VERSION)
        .then_some(Delivery::Installed))
}

fn text(url: &str) -> Result<String> {
    Ok(velopack::download::download_url_as_string(
        url,
        Some(&HttpOptions {
            TimeoutMilliseconds: 10_000,
            ..Default::default()
        }),
    )?)
}

pub(crate) fn check() -> Result<Check> {
    check_from(None)
}

#[cfg(windows)]
pub(crate) fn check_confirmed(tag: &str) -> Result<Check> {
    ensure!(is_stable_tag(tag), "Invalid confirmed update tag.");
    check_from(Some(tag))
}

fn check_from(confirmed_tag: Option<&str>) -> Result<Check> {
    let Some(channel) = platform_channel() else {
        return Ok(Check::Unsupported);
    };
    let Some(mode) = delivery(&std::env::current_exe()?, channel)? else {
        return Ok(Check::Unsupported);
    };
    if confirmed_tag.is_some() && mode != Delivery::Installed {
        return Ok(Check::Unsupported);
    }
    check_feed(
        RELEASES,
        channel,
        mode,
        crate::constants::VERSION,
        confirmed_tag,
    )
}

fn check_feed(
    releases: &str,
    channel: &str,
    mode: Delivery,
    current: &str,
    confirmed_tag: Option<&str>,
) -> Result<Check> {
    let base = confirmed_tag.map_or_else(
        || format!("{releases}/latest/download"),
        |tag| format!("{releases}/download/{tag}"),
    );
    let body = text(&format!("{base}/releases.{channel}.json"))?;
    select_target(&body, channel, mode, current, confirmed_tag)
}

fn select_target(
    body: &str,
    channel: &str,
    delivery: Delivery,
    current: &str,
    confirmed_tag: Option<&str>,
) -> Result<Check> {
    let feed: VelopackAssetFeed = serde_json::from_str(body)?;
    let [asset]: [VelopackAsset; 1] = feed
        .Assets
        .try_into()
        .ok()
        .context("Expected one full release in the stable feed.")?;
    let version = stable_version(&asset.Version)?;
    let suffix = if channel == "win" {
        String::new()
    } else {
        format!("-{channel}")
    };
    ensure!(
        asset.Type == "Full"
            && asset.PackageId == "TapRelay"
            && asset.FileName == format!("TapRelay-{version}{suffix}-full.nupkg")
            && asset.Size > 0
            && asset.SHA256.len() == 64
            && asset.SHA256.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid TapRelay full release for channel {channel}."
    );
    let target = Target {
        asset: Arc::new(asset),
        delivery,
    };
    ensure!(
        confirmed_tag.is_none_or(|tag| tag == target.tag()),
        "Release feed does not match the confirmed tag."
    );
    Ok(if version > stable_version(current)? {
        Check::Available(target)
    } else {
        Check::Current
    })
}

#[cfg(test)]
mod tests;
