use super::protocol::{self, Command, Message};
use crate::platform::native::bluetooth::{BleHandle, Request};
use anyhow::{Result, bail};
use std::{
    io::BufReader,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use taprelay_core::{
    command::{COMMAND_TTL, CommandPhase, QueuedCommand},
    passthrough::MAX_INPUT_AGE,
};

pub fn run() -> Result<()> {
    let mut stdin = BufReader::new(std::io::stdin());
    let mut stdout = std::io::stdout();
    let Some(initial) = protocol::read::<Command>(&mut stdin)? else {
        return Ok(());
    };
    let Command::Init {
        profile,
        publish,
        remembered,
        selected,
    } = initial
    else {
        bail!("Bluetooth worker requires initialization");
    };
    let handle = BleHandle::start(remembered, profile, publish)?;
    if let Some(selected) = selected {
        handle.request(Request::Select(selected))?;
        handle.invalidate_commands();
    }
    let link = handle.input_link();
    let (tx, commands) = mpsc::sync_channel(1024);
    thread::Builder::new()
        .name("bluetooth-worker-reader".into())
        .spawn(move || {
            while let Ok(Some(command)) = protocol::read::<Command>(&mut stdin) {
                if tx.send(command).is_err() {
                    break;
                }
            }
        })?;
    let mut last_status = None;
    let mut progress_at = Instant::now();
    let mut pending = Vec::new();
    'worker: loop {
        #[cfg(target_os = "macos")]
        handle.poll();
        let Some(batch) = command_batch(&commands) else {
            break;
        };
        for command in batch {
            match command {
                #[cfg(target_os = "macos")]
                Command::SetProfile(profile) => handle.set_profile(profile),
                Command::Shutdown => break 'worker,
                Command::Refresh => handle.request(Request::Refresh)?,
                Command::Pair(id) => handle.request(Request::Pair(id))?,
                Command::Invalidate => {
                    handle.invalidate_commands();
                    link.set_mapping_profile(false);
                    link.end();
                }
                Command::Arm { generation } => {
                    let accepted = generation == link.generation() && link.ready();
                    if accepted {
                        link.set_mapping_profile(true);
                    }
                    protocol::write(
                        &Message::Armed {
                            generation,
                            accepted,
                        },
                        &mut stdout,
                    )?;
                }
                Command::Input {
                    generation,
                    captured_ms,
                    event,
                    mouse_percent,
                    mouse_report_rate,
                    reverse_scroll,
                } => {
                    if generation == link.generation() && link.mapping_epoch() != 0 {
                        if let Some(age) =
                            protocol::age(captured_ms).filter(|age| *age <= MAX_INPUT_AGE)
                        {
                            link.set_mouse_percent(mouse_percent);
                            link.set_mouse_report_rate(mouse_report_rate);
                            link.set_reverse_scroll(reverse_scroll);
                            link.submit_forwarded(event, Instant::now() - age);
                        } else {
                            link.fail("Bluetooth worker input exceeded 250 ms");
                        }
                    }
                }
                Command::Media {
                    id,
                    generation,
                    captured_ms,
                    target,
                    action,
                    down,
                } => {
                    let age = protocol::age(captured_ms).filter(|age| {
                        generation == link.generation()
                            && (!down || *age <= COMMAND_TTL)
                            && pending.len() < 32
                    });
                    if let Some(age) = age {
                        let (reply, result) = tokio::sync::oneshot::channel();
                        let command = QueuedCommand {
                            action,
                            phase: if down {
                                CommandPhase::Press
                            } else {
                                CommandPhase::Release
                            },
                            target,
                            generation,
                            created: Instant::now().checked_sub(age).unwrap_or_else(Instant::now),
                        };
                        match handle.request(Request::Send(command, reply)) {
                            Ok(()) => pending.push((id, result)),
                            Err(error) => protocol::write(
                                &Message::Reply {
                                    id,
                                    result: Err(error.into()),
                                },
                                &mut stdout,
                            )?,
                        }
                    } else {
                        protocol::write(
                            &Message::Reply {
                                id,
                                result: Err(protocol::Error::Stale),
                            },
                            &mut stdout,
                        )?;
                    }
                }
                Command::Init { .. } => {
                    bail!("Duplicate Bluetooth worker initialization")
                }
            }
        }
        let mut index = 0;
        while index < pending.len() {
            let result = match pending[index].1.try_recv() {
                Ok(result) => Some(result.map_err(Into::into)),
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Some(Err(
                    protocol::Error::Failed("Bluetooth sender stopped".into()),
                )),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let (id, _) = pending.swap_remove(index);
                protocol::write(&Message::Reply { id, result }, &mut stdout)?;
            } else {
                index += 1;
            }
        }
        let mut state = handle.state.borrow().clone();
        state.ready &= state.generation == link.generation();
        let input_available = link.ready() || link.epoch() != 0;
        if last_status.as_ref() != Some(&(state.clone(), input_available)) {
            protocol::write(
                &Message::Status {
                    state: Box::new(state.clone()),
                    input_available,
                },
                &mut stdout,
            )?;
            last_status = Some((state, input_available));
        }
        if let Some(error) = link.take_failure() {
            protocol::write(&Message::Failure(error), &mut stdout)?;
        }
        #[cfg(windows)]
        if handle.is_finished() {
            bail!("Native Bluetooth worker stopped");
        }
        if Instant::now() >= progress_at {
            protocol::write(&Message::NativeProgress(handle.progress()), &mut stdout)?;
            progress_at = Instant::now() + Duration::from_secs(1);
        }
    }
    link.set_mapping_profile(false);
    link.end();
    drop(handle);
    Ok(())
}

// Pump Core Bluetooth once per bounded IPC batch, not once per mouse sample.
// The scheduler can then coalesce unsent motion without accumulating 1 ms of
// run-loop waiting for every event from a high-polling-rate mouse.
fn command_batch(commands: &mpsc::Receiver<Command>) -> Option<impl Iterator<Item = Command> + '_> {
    let first = match commands.recv_timeout(Duration::from_millis(5)) {
        Ok(command) => Some(command),
        Err(mpsc::RecvTimeoutError::Timeout) => None,
        Err(mpsc::RecvTimeoutError::Disconnected) => return None,
    };
    #[cfg(target_os = "macos")]
    const LIMIT: usize = 256;
    #[cfg(windows)]
    const LIMIT: usize = 1;
    // Windows still handles one command per iteration. Neither platform
    // allocates a temporary batch or blocks while draining the remaining work.
    let remaining = if first.is_some() { LIMIT - 1 } else { 0 };
    Some(first.into_iter().chain(commands.try_iter().take(remaining)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_batch_is_empty_and_disconnection_ends_the_worker() {
        let (tx, rx) = mpsc::channel();
        assert_eq!(command_batch(&rx).unwrap().count(), 0);
        drop(tx);
        assert!(command_batch(&rx).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn windows_keeps_one_command_per_iteration() {
        let (tx, rx) = mpsc::channel();
        tx.send(Command::Refresh).unwrap();
        tx.send(Command::Shutdown).unwrap();
        let mut batch = command_batch(&rx).unwrap();
        assert!(matches!(batch.next(), Some(Command::Refresh)));
        assert!(batch.next().is_none());
        assert!(matches!(
            command_batch(&rx).unwrap().next(),
            Some(Command::Shutdown)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_loop_drains_bursts_in_bounded_batches_without_reordering_shutdown() {
        let (tx, rx) = mpsc::channel();
        for id in 0..512 {
            tx.send(Command::Arm { generation: id }).unwrap();
        }
        tx.send(Command::Shutdown).unwrap();
        drop(tx);
        for start in [0, 256] {
            let batch: Vec<_> = command_batch(&rx).unwrap().collect();
            assert_eq!(batch.len(), 256);
            for (index, command) in batch.into_iter().enumerate() {
                assert!(
                    matches!(command,Command::Arm {generation} if generation==start+index as u64)
                );
            }
        }
        assert!(matches!(
            command_batch(&rx).unwrap().collect::<Vec<_>>().as_slice(),
            [Command::Shutdown]
        ));
        assert!(command_batch(&rx).is_none());
    }
}
