use crate::platform::Transport;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use taprelay_core::{
    command::QueuedCommand,
    devices::{AdapterState, Availability, Connection},
    ports::BackendError,
    state::{Knowledge, Snapshot, Target},
};
use tokio::sync::oneshot;

#[derive(Debug, PartialEq, Eq)]
pub(in crate::gui::view::tests) enum Call {
    Select(String),
    Pair(String),
    Disconnect,
    Refresh,
    Send,
}

#[derive(Clone)]
pub(in crate::gui::view::tests) struct Receiver {
    pub state: Arc<Mutex<Snapshot>>,
    pub calls: Arc<Mutex<Vec<Call>>>,
}

impl Receiver {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(Snapshot {
                adapter_state: AdapterState::Available,
                adapter: true,
                peripheral: true,
                service: true,
                broadcasting: true,
                targets: vec![Target {
                    id: "tablet".into(),
                    name: "Tablet".into(),
                    pairing: Knowledge::Yes,
                    link: Knowledge::Yes,
                    subscribed: Knowledge::Yes,
                    availability: Availability::Nearby,
                    ..Default::default()
                }],
                ..Default::default()
            })),
            calls: Default::default(),
        }
    }

    pub fn connected(&self) {
        let mut state = self.state.lock().unwrap();
        assert!(state.selected.is_some());
        let selected = state.selected.clone();
        for target in &mut state.targets {
            if Some(&target.id) == selected.as_ref() {
                target.connection = Connection::Connected;
            }
        }
        state.ready = true;
    }
}

impl Transport for Receiver {
    fn snapshot(&mut self) -> Option<Snapshot> {
        Some(self.state.lock().unwrap().clone())
    }

    fn select(&self, id: String) -> Result<u64> {
        self.calls.lock().unwrap().push(Call::Select(id.clone()));
        let mut state = self.state.lock().unwrap();
        state.selected = Some(id);
        state.ready = false;
        Ok(state.generation)
    }

    fn pair(&self, id: String) -> Result<()> {
        self.calls.lock().unwrap().push(Call::Pair(id));
        Ok(())
    }

    fn disconnect(&self) -> Result<u64> {
        self.calls.lock().unwrap().push(Call::Disconnect);
        let mut state = self.state.lock().unwrap();
        state.selected = None;
        state.ready = false;
        for target in &mut state.targets {
            target.connection = Connection::Disconnected;
        }
        Ok(state.generation)
    }

    fn refresh(&self) -> Result<()> {
        self.calls.lock().unwrap().push(Call::Refresh);
        Ok(())
    }

    fn restart(&self) -> Result<u64> {
        Ok(self.state.lock().unwrap().generation)
    }

    fn invalidate(&self) {}

    fn send(&self, _: QueuedCommand) -> Result<oneshot::Receiver<Result<(), BackendError>>> {
        self.calls.lock().unwrap().push(Call::Send);
        let (tx, rx) = oneshot::channel();
        tx.send(Ok(())).unwrap();
        Ok(rx)
    }
}
