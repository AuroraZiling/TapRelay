use crate::logging;
use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrivilegeMode {
    Config,
    User,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StartupOptions {
    pub(super) handoff: Option<u32>,
    #[cfg(windows)]
    pub(super) confirmed_update: Option<(String, String)>,
    pub(super) privilege: PrivilegeMode,
    pub(super) log_level: logging::LogLevel,
}

pub(super) fn parse_startup_options(args: &[String]) -> Result<StartupOptions> {
    let mut options = StartupOptions {
        handoff: None,
        #[cfg(windows)]
        confirmed_update: None,
        privilege: PrivilegeMode::Config,
        log_level: logging::LogLevel::INFO,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--user" => {
                if options.privilege == PrivilegeMode::Admin {
                    bail!("--user and --admin cannot be used together");
                }
                options.privilege = PrivilegeMode::User;
            }
            "--admin" => {
                if options.privilege == PrivilegeMode::User {
                    bail!("--user and --admin cannot be used together");
                }
                options.privilege = PrivilegeMode::Admin;
            }
            "--handoff" => {
                let value = args
                    .get(index + 1)
                    .context("--handoff requires a process id")?;
                options.handoff = Some(
                    value
                        .parse::<u32>()
                        .with_context(|| format!("Invalid --handoff process id `{value}`"))?,
                );
                index += 1;
            }
            #[cfg(windows)]
            "--confirmed-update" => {
                let tag = args
                    .get(index + 1)
                    .context("Missing confirmed release tag")?;
                anyhow::ensure!(
                    crate::updates::is_stable_tag(tag),
                    "Expected a stable release tag"
                );
                let nonce = args.get(index + 2).context("Missing update handoff ID")?;
                options.confirmed_update = Some((tag.clone(), nonce.clone()));
                index += 2;
            }
            "--log-level" => {
                let value = args
                    .get(index + 1)
                    .context("--log-level requires a level")?;
                options.log_level = logging::parse_level(value)?;
                index += 1;
            }
            argument if argument.starts_with("--log-level=") => {
                options.log_level = logging::parse_level(&argument["--log-level=".len()..])?;
            }
            argument => {
                bail!(
                    "Unknown argument `{argument}`. Supported arguments: --user, --admin, --log-level <level>."
                )
            }
        }
        index += 1;
    }
    #[cfg(windows)]
    anyhow::ensure!(
        options.confirmed_update.is_none()
            || (options.privilege == PrivilegeMode::User
                && options.handoff.is_some_and(|pid| pid != 0)),
        "Confirmed update requires an explicit user-mode handoff"
    );
    Ok(options)
}

pub(super) fn should_request_admin(
    privilege: PrivilegeMode,
    configured_always_admin: bool,
) -> bool {
    match privilege {
        PrivilegeMode::User => false,
        PrivilegeMode::Admin => true,
        PrivilegeMode::Config => configured_always_admin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn defaults_to_configured_privilege_mode() {
        assert_eq!(
            parse_startup_options(&args(&[])).unwrap(),
            StartupOptions {
                handoff: None,
                #[cfg(windows)]
                confirmed_update: None,
                privilege: PrivilegeMode::Config,
                log_level: logging::LogLevel::INFO,
            }
        );
    }

    #[test]
    fn explicit_user_and_admin_modes_override_config() {
        assert_eq!(
            parse_startup_options(&args(&["--user"])).unwrap().privilege,
            PrivilegeMode::User
        );
        assert_eq!(
            parse_startup_options(&args(&["--admin"]))
                .unwrap()
                .privilege,
            PrivilegeMode::Admin
        );
        assert!(!should_request_admin(PrivilegeMode::User, true));
        assert!(should_request_admin(PrivilegeMode::Admin, false));
        assert!(should_request_admin(PrivilegeMode::Config, true));
        assert!(!should_request_admin(PrivilegeMode::Config, false));
    }

    #[test]
    fn parses_private_handoff_argument() {
        assert_eq!(
            parse_startup_options(&args(&["--handoff", "1234"])).unwrap(),
            StartupOptions {
                handoff: Some(1234),
                #[cfg(windows)]
                confirmed_update: None,
                privilege: PrivilegeMode::Config,
                log_level: logging::LogLevel::INFO,
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn confirmed_update_requires_fixed_stable_tag_and_explicit_user_process_handoff() {
        let valid = [
            "--user",
            "--handoff",
            "1234",
            "--confirmed-update",
            "v1.2.3",
            "nonce",
        ];
        let parsed = parse_startup_options(&args(&valid)).unwrap();
        assert_eq!(
            parsed.confirmed_update,
            Some(("v1.2.3".into(), "nonce".into()))
        );
        for flags in [
            &valid[1..],
            &valid[..5],
            &["--user", "--confirmed-update", "v1.2.3", "nonce"],
            &[
                "--admin",
                "--handoff",
                "1234",
                "--confirmed-update",
                "v1.2.3",
                "nonce",
            ],
            &[
                "--user",
                "--handoff",
                "0",
                "--confirmed-update",
                "v1.2.3",
                "nonce",
            ],
            &[
                "--user",
                "--handoff",
                "1234",
                "--confirmed-update",
                "latest",
                "nonce",
            ],
        ] {
            assert!(parse_startup_options(&args(flags)).is_err(), "{flags:?}");
        }
    }

    #[test]
    fn rejects_conflicting_or_unknown_arguments() {
        assert!(parse_startup_options(&args(&["--user", "--admin"])).is_err());
        assert!(parse_startup_options(&args(&["--unexpected"])).is_err());
        assert!(parse_startup_options(&args(&["--handoff"])).is_err());
    }

    #[test]
    fn log_level_accepts_both_forms_and_survives_handoff_arguments() {
        for (name, level) in [
            ("off", logging::LogLevel::OFF),
            ("error", logging::LogLevel::ERROR),
            ("warn", logging::LogLevel::WARN),
            ("info", logging::LogLevel::INFO),
            ("debug", logging::LogLevel::DEBUG),
            ("trace", logging::LogLevel::TRACE),
        ] {
            let options = parse_startup_options(&args(&["--user", "--log-level", name])).unwrap();
            assert_eq!(options.log_level, level);
            let options = parse_startup_options(&args(&[
                "--handoff",
                "1234",
                &format!("--log-level={}", name.to_ascii_uppercase()),
            ]))
            .unwrap();
            assert_eq!(options.log_level, level);
            assert_eq!(options.handoff, Some(1234));
        }
    }

    #[test]
    fn invalid_or_missing_log_level_is_rejected() {
        for values in [
            vec!["--log-level"],
            vec!["--log-level="],
            vec!["--log-level", "verbose"],
            vec!["--log-level", "--user"],
        ] {
            assert!(parse_startup_options(&args(&values)).is_err());
        }
    }
}
