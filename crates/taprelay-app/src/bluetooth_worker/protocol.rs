use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{BufRead, Read, Write};
use taprelay_core::{
    command::MediaCommand,
    hid::Profile,
    passthrough::Event,
    state::{Snapshot, Target},
};

const FRAME_LIMIT: u64 = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub enum Command {
    Init {
        profile: Profile,
        publish: bool,
        remembered: Option<Target>,
        selected: Option<String>,
    },
    Pair(String),
    Refresh,
    Invalidate,
    Arm {
        generation: u64,
    },
    Input {
        generation: u64,
        captured_ms: u64,
        event: Event,
        mouse_percent: u16,
        reverse_scroll: bool,
    },
    Media {
        id: u64,
        generation: u64,
        captured_ms: u64,
        target: String,
        action: MediaCommand,
        down: bool,
    },
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Error {
    Stale,
    Failed(String),
}

impl From<taprelay_core::ports::BackendError> for Error {
    fn from(error: taprelay_core::ports::BackendError) -> Self {
        match error {
            taprelay_core::ports::BackendError::Stale => Self::Stale,
            error => Self::Failed(error.to_string()),
        }
    }
}

impl From<Error> for taprelay_core::ports::BackendError {
    fn from(error: Error) -> Self {
        match error {
            Error::Stale => Self::Stale,
            Error::Failed(message) => Self::Unavailable(message),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    Status {
        state: Box<Snapshot>,
        input_available: bool,
    },
    Armed {
        generation: u64,
        accepted: bool,
    },
    Reply {
        id: u64,
        result: Result<(), Error>,
    },
    Failure(String),
}

pub fn read<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<Option<T>> {
    let mut bytes = Vec::new();
    let count = reader.take(FRAME_LIMIT + 1).read_until(b'\n', &mut bytes)?;
    if count == 0 {
        return Ok(None);
    }
    ensure!(
        count as u64 <= FRAME_LIMIT && bytes.last() == Some(&b'\n'),
        "Invalid Bluetooth worker frame"
    );
    Ok(Some(
        serde_json::from_slice(&bytes).context("Decode Bluetooth worker frame")?,
    ))
}

pub fn write(value: &impl Serialize, writer: &mut impl Write) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub fn captured_ms(captured: std::time::Instant) -> u64 {
    taprelay_windows::monotonic_millis()
        .saturating_sub(captured.elapsed().as_millis().min(u64::MAX as u128) as u64)
}

pub fn age(captured_ms: u64) -> Option<std::time::Duration> {
    taprelay_windows::monotonic_millis()
        .checked_sub(captured_ms)
        .map(std::time::Duration::from_millis)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_preserve_profile_identity_and_unicode() {
        let command = Command::Init {
            profile: Profile::MediaOnly,
            publish: true,
            remembered: Some(Target {
                id: "手机".into(),
                ..Default::default()
            }),
            selected: None,
        };
        let mut bytes = Vec::new();
        write(&command, &mut bytes).unwrap();
        let Some(Command::Init {
            profile,
            remembered,
            ..
        }) = read(&mut bytes.as_slice()).unwrap()
        else {
            panic!("wrong frame");
        };
        assert_eq!(profile, Profile::MediaOnly);
        assert_eq!(remembered.unwrap().id, "手机");
    }

    #[test]
    fn truncated_and_oversized_frames_are_rejected() {
        assert!(read::<Command>(&mut b"\"Shutdown\"".as_slice()).is_err());
        assert!(read::<Command>(&mut vec![b' '; FRAME_LIMIT as usize + 2].as_slice()).is_err());
        assert!(read::<Command>(&mut b"".as_slice()).unwrap().is_none());
    }
}
