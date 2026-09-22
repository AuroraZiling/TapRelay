use super::protocol::{self, Command, Message};
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
use taprelay_windows::bluetooth::{BleHandle, Request};

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
    let mut pending = Vec::new();
    loop {
        match commands.recv_timeout(Duration::from_millis(5)) {
            Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Ok(Command::Refresh) => handle.request(Request::Refresh)?,
            Ok(Command::Pair(id)) => handle.request(Request::Pair(id))?,
            Ok(Command::Invalidate) => {
                handle.invalidate_commands();
                link.end();
            }
            Ok(Command::Arm { generation }) => {
                let accepted = generation == link.generation() && link.begin();
                protocol::write(
                    &Message::Armed {
                        generation,
                        accepted,
                    },
                    &mut stdout,
                )?;
            }
            Ok(Command::Input {
                generation,
                captured_ms,
                event,
                mouse_percent,
                reverse_scroll,
            }) => {
                if generation == link.generation() && link.epoch() != 0 {
                    if let Some(age) =
                        protocol::age(captured_ms).filter(|age| *age <= MAX_INPUT_AGE)
                    {
                        link.set_mouse_percent(mouse_percent);
                        link.set_reverse_scroll(reverse_scroll);
                        link.submit(event, Instant::now() - age);
                    } else {
                        link.fail("Bluetooth worker input exceeded 250 ms");
                    }
                }
            }
            Ok(Command::Media {
                id,
                generation,
                captured_ms,
                target,
                action,
                down,
            }) => {
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
            Ok(Command::Init { .. }) => {
                bail!("Duplicate Bluetooth worker initialization")
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
        if handle.is_finished() {
            bail!("Native Bluetooth worker stopped");
        }
    }
    link.end();
    drop(handle);
    Ok(())
}
