use std::{
    fs,
    sync::{Arc, mpsc::Sender},
};

use anyhow::{Result, ensure};
use velopack::{
    HttpOptions, UpdateInfo, UpdateManager, VelopackAsset,
    locator::{self, LocationContext, VelopackLocator},
    sources::HttpSource,
};

use super::{Delivery, RELEASES, Target, platform_channel, stable_version};

pub(crate) struct PreparedUpdate {
    manager: UpdateManager,
    asset: Arc<VelopackAsset>,
}

impl PreparedUpdate {
    pub(crate) fn apply(&self) -> Result<()> {
        require_standard_user()?;
        self.manager.wait_exit_then_apply_updates(
            &*self.asset,
            false,
            true,
            Vec::<String>::new(),
        )?;
        Ok(())
    }
}

fn require_standard_user() -> Result<()> {
    #[cfg(windows)]
    ensure!(
        !crate::platform::administrator::status()?.elevated,
        "Updates require the non-elevated user session."
    );
    Ok(())
}

pub(super) fn source(releases: &str, target: &Target) -> HttpSource {
    HttpSource::new_with_options(
        format!("{releases}/download/{}/", target.tag()),
        HttpOptions {
            TimeoutMilliseconds: 900_000,
            ..Default::default()
        },
    )
}

pub(crate) fn download(target: &Target, progress: Sender<i16>) -> Result<PreparedUpdate> {
    ensure!(
        target.delivery == Delivery::Installed,
        "A portable ZIP is downloaded in the browser."
    );
    require_standard_user()?;
    let locator = locator::auto_locate_app_manifest(LocationContext::FromCurrentExe)?;
    ensure!(
        locator.get_manifest_id() == "TapRelay"
            && Some(locator.get_manifest_channel().as_str()) == platform_channel()
            && locator.get_manifest_version() < stable_version(target.version())?,
        "The installed app no longer matches the offered update."
    );
    let manager = UpdateManager::new(source(RELEASES, target), None, None)?;
    download_checked(manager, &locator, target, progress)
}

pub(super) fn download_checked(
    manager: UpdateManager,
    locator: &VelopackLocator,
    target: &Target,
    progress: Sender<i16>,
) -> Result<PreparedUpdate> {
    {
        // Velopack 1.2.161 skips its checksum check if the final file already exists.
        let _lock = locator.try_get_exclusive_lock()?;
        let cached = locator.get_packages_dir().join(&target.asset.FileName);
        if cached.try_exists()?
            && (fs::metadata(&cached)?.len() != target.asset.Size
                || !velopack::misc::calculate_sha1_sha256(&cached)?
                    .1
                    .eq_ignore_ascii_case(&target.asset.SHA256))
        {
            fs::remove_file(cached)?;
        }
    }
    let update = UpdateInfo {
        TargetFullRelease: (*target.asset).clone(),
        ..Default::default()
    };
    manager.download_updates(&update, Some(progress.clone()))?;
    let _ = progress.send(100);
    Ok(PreparedUpdate {
        manager,
        asset: target.asset.clone(),
    })
}
