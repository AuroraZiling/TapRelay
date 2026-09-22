use super::{
    Execution, WorkerProcess,
    protocol::{self, Command, Message},
};
use anyhow::{Context, Result, bail};
use std::{
    io::BufReader,
    os::windows::process::CommandExt,
    process::{Child, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub(super) struct Native;

impl Execution for Native {
    type Worker = Worker;

    fn start(&mut self, initial: Command) -> Result<Worker> {
        Worker::start(initial)
    }

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn wait(&mut self, duration: Duration) {
        thread::park_timeout(duration);
    }
}

pub(super) struct Worker {
    child: Child,
    outgoing: Option<mpsc::SyncSender<Command>>,
    incoming: mpsc::Receiver<Result<Message>>,
    threads: Vec<thread::JoinHandle<()>>,
    exited: bool,
    stopping: Arc<AtomicBool>,
}

impl Worker {
    pub fn start(initial: Command) -> Result<Self> {
        Self::start_command(
            std::process::Command::new(std::env::current_exe()?).arg("--bluetooth-worker"),
            initial,
        )
    }

    fn start_command(command: &mut std::process::Command, initial: Command) -> Result<Self> {
        let child = command
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Start isolated Bluetooth worker")?;
        let (outgoing, commands) = mpsc::sync_channel(1024);
        let (messages, incoming) = mpsc::sync_channel(128);
        let mut worker = Self {
            child,
            outgoing: Some(outgoing),
            incoming,
            threads: Vec::new(),
            exited: false,
            stopping: Arc::new(AtomicBool::new(false)),
        };
        let mut stdin = worker
            .child
            .stdin
            .take()
            .context("Bluetooth worker stdin")?;
        let stdout = worker
            .child
            .stdout
            .take()
            .context("Bluetooth worker stdout")?;
        let stderr = worker
            .child
            .stderr
            .take()
            .context("Bluetooth worker stderr")?;
        let failures = messages.clone();
        let stopping = worker.stopping.clone();
        worker.threads.push(
            thread::Builder::new()
                .name("bluetooth-worker-writer".into())
                .spawn(move || {
                    loop {
                        if stopping.load(Ordering::Acquire) {
                            let _ = protocol::write(&Command::Shutdown, &mut stdin);
                            break;
                        }
                        let command = match commands.recv_timeout(Duration::from_millis(5)) {
                            Ok(command) => command,
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                let _ = protocol::write(&Command::Shutdown, &mut stdin);
                                break;
                            }
                        };
                        if stopping.load(Ordering::Acquire) {
                            continue;
                        }
                        if let Err(error) = protocol::write(&command, &mut stdin) {
                            let _ = failures.try_send(Err(error));
                            break;
                        }
                    }
                })?,
        );
        worker.threads.push(
            thread::Builder::new()
                .name("bluetooth-worker-status".into())
                .spawn(move || {
                    let mut reader = BufReader::new(stdout);
                    loop {
                        match protocol::read::<Message>(&mut reader) {
                            Ok(Some(message)) => {
                                if messages.send(Ok(message)).is_err() {
                                    break;
                                }
                            }
                            Ok(None) => {
                                let _ = messages
                                    .try_send(Err(anyhow::anyhow!("Bluetooth worker pipe closed")));
                                break;
                            }
                            Err(error) => {
                                let _ = messages.try_send(Err(error));
                                break;
                            }
                        }
                    }
                })?,
        );
        worker.threads.push(
            thread::Builder::new()
                .name("bluetooth-worker-log".into())
                .spawn(move || {
                    use std::io::BufRead;
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        tracing::info!(worker = %line, "Bluetooth worker");
                    }
                })?,
        );
        worker.send(initial)?;
        tracing::info!(pid = worker.child.id(), "Isolated Bluetooth worker started");
        Ok(worker)
    }

    pub fn send(&self, command: Command) -> Result<()> {
        self.outgoing
            .as_ref()
            .context("Bluetooth worker stopped")?
            .try_send(command)
            .map_err(|_| anyhow::anyhow!("Bluetooth worker queue unavailable"))
    }

    pub fn stop(&mut self) -> Result<()> {
        if self.exited {
            return Ok(());
        }
        self.stopping.store(true, Ordering::Release);
        self.outgoing.take();
        let until = Instant::now() + Duration::from_millis(400);
        while self.child.try_wait()?.is_none() && Instant::now() < until {
            while self.incoming.try_recv().is_ok() {}
            thread::sleep(Duration::from_millis(5));
        }
        if self.child.try_wait()?.is_none() {
            self.child
                .kill()
                .context("Terminate Bluetooth service owner")?;
        }
        let status = self
            .child
            .wait()
            .context("Wait for Bluetooth service owner to exit")?;
        self.exited = true;
        tracing::info!(
            pid = self.child.id(),
            ?status,
            "Bluetooth service owner exited"
        );
        for handle in self.threads.drain(..) {
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
        Ok(())
    }

    pub fn check(&mut self) -> Result<()> {
        if let Some(status) = self.child.try_wait()? {
            bail!("Bluetooth worker exited: {status}");
        }
        Ok(())
    }
}

impl WorkerProcess for Worker {
    fn send(&self, command: Command) -> Result<()> {
        Self::send(self, command)
    }

    fn receive(&self) -> Result<Option<Message>> {
        match self.incoming.try_recv() {
            Ok(message) => message.map(Some),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => bail!("Bluetooth worker reader stopped"),
        }
    }

    fn check(&mut self) -> Result<()> {
        Self::check(self)
    }

    fn stop(&mut self) -> Result<()> {
        Self::stop(self)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            tracing::error!(%error, "Bluetooth worker cleanup failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(script: &str) -> Worker {
        let mut command = std::process::Command::new("powershell.exe");
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ]);
        let worker = Worker::start_command(&mut command, Command::Refresh).unwrap();
        let message = worker
            .incoming
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert!(matches!(message, Message::Failure(message) if message == "probe-ready"));
        worker
    }

    #[test]
    fn native_process_exchanges_frames_and_exits_normally_while_status_queue_is_full() {
        let mut worker = probe(
            r#"
            $null = [Console]::In.ReadLine()
            [Console]::Out.WriteLine('{"Failure":"probe-ready"}')
            while ($null -ne ($line = [Console]::In.ReadLine())) {
                if ($line -eq '"Shutdown"') {
                    for ($i = 0; $i -lt 512; $i++) {
                        [Console]::Out.WriteLine('{"Failure":"final-status"}')
                    }
                    exit 0
                }
            }
            exit 1
        "#,
        );
        worker.check().unwrap();
        worker.stop().unwrap();
        assert!(worker.exited);
        assert!(worker.child.try_wait().unwrap().unwrap().success());
        worker.stop().unwrap();
        assert!(worker.send(Command::Refresh).is_err());
    }

    #[test]
    fn native_process_that_ignores_shutdown_is_killed_and_reaped() {
        let mut worker = probe(
            r#"
            $null = [Console]::In.ReadLine()
            [Console]::Out.WriteLine('{"Failure":"probe-ready"}')
            Start-Sleep -Seconds 60
            exit 0
        "#,
        );
        worker.stop().unwrap();
        assert!(worker.exited);
        assert!(!worker.child.try_wait().unwrap().unwrap().success());
        assert!(worker.check().is_err());
    }

    #[test]
    fn native_process_spawn_failure_is_reported() {
        let missing = std::env::temp_dir().join(format!(
            "taprelay-missing-worker-{}.exe",
            std::process::id()
        ));
        let result =
            Worker::start_command(&mut std::process::Command::new(missing), Command::Refresh);
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("Start isolated Bluetooth worker")
        );
    }
}
