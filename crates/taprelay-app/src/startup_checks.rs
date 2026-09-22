use crate::{
    CheckRow,
    i18n::{self, keys},
};
use std::time::Duration;

pub fn should_present(passed: bool, failed: bool, elapsed: Duration) -> bool {
    passed || failed || elapsed >= Duration::from_millis(300)
}

pub fn rows(
    locale: &str,
    environment_failed: bool,
    flags: [bool; 4],
    failed: bool,
) -> Vec<CheckRow> {
    let flags = if environment_failed {
        [false; 5]
    } else {
        [true, flags[0], flags[1], flags[2], flags[3]]
    };
    let first = flags.iter().position(|passed| !passed).unwrap_or(5);
    [
        keys::CHECK_ENVIRONMENT,
        keys::CHECK_BLUETOOTH,
        keys::CHECK_PERIPHERAL,
        keys::CHECK_SERVICE,
        keys::CHECK_ADVERTISING,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, key)| CheckRow {
        title: i18n::text(locale, key).into(),
        state: if flags[index] {
            2
        } else if index == first {
            if environment_failed || failed { 3 } else { 1 }
        } else {
            0
        },
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_failure_leaves_bluetooth_checks_waiting() {
        let rows = rows("zh-cn", true, [false; 4], true);
        assert_eq!(
            rows.iter().map(|row| row.state).collect::<Vec<_>>(),
            [3, 0, 0, 0, 0]
        );
        assert_eq!(rows[0].title, "环境");
    }

    #[test]
    fn bluetooth_failure_preserves_passed_environment_and_prior_checks() {
        for first in 0..4 {
            let flags = std::array::from_fn(|index| index < first);
            let rows = rows("en", false, flags, true);
            assert_eq!(rows[0].title, "Environment");
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(
                    row.state,
                    if index <= first {
                        2
                    } else if index == first + 1 {
                        3
                    } else {
                        0
                    }
                );
            }
        }
    }

    #[test]
    fn environment_passes_before_bluetooth_starts_and_all_checks_can_pass() {
        assert_eq!(
            rows("en", false, [false; 4], false)
                .iter()
                .map(|row| row.state)
                .collect::<Vec<_>>(),
            [2, 1, 0, 0, 0]
        );
        assert!(
            rows("en", false, [true; 4], false)
                .iter()
                .all(|row| row.state == 2)
        );
    }
}
