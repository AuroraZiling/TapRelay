//! Capture ownership is changed only by the run loop. Callbacks may revoke
//! admission immediately, but keep the cursor lease alive until cleanup runs.
use std::time::Instant;
use taprelay_core::passthrough::InputLink;

pub(super) trait CursorLease {
    fn ready(&mut self, ours: bool, now: Instant) -> Result<bool, &'static str>;
}

enum State<C> {
    Idle,
    Preparing(C),
    Capturing { cursor: C, since: Instant },
    Releasing(C),
    Faulted,
}

pub(super) struct Session<C> {
    state: State<C>,
    fault: Option<&'static str>,
}

impl<C> Default for Session<C> {
    fn default() -> Self {
        Self {
            state: State::Idle,
            fault: None,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Context {
    pub now: Instant,
    pub ours: bool,
    pub permitted: bool,
    pub session_active: bool,
    pub secure_input: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Transition {
    Unchanged,
    Started,
    Stopped,
    Failed(&'static str),
}

impl<C: CursorLease> Session<C> {
    pub fn started(&self) -> Option<Instant> {
        match self.state {
            State::Capturing { since, .. } => Some(since),
            _ => None,
        }
    }

    pub fn owns_cursor(&self) -> bool {
        matches!(
            self.state,
            State::Preparing(_) | State::Capturing { .. } | State::Releasing(_)
        )
    }

    /// Safe inside native callbacks
    pub fn stop(&mut self) {
        self.state = match std::mem::replace(&mut self.state, State::Idle) {
            State::Preparing(cursor)
            | State::Capturing { cursor, .. }
            | State::Releasing(cursor) => State::Releasing(cursor),
            state => state,
        };
    }

    pub fn release(&mut self) -> Transition {
        if !matches!(self.state, State::Releasing(_)) {
            return Transition::Unchanged;
        }
        self.state = if self.fault.is_some() {
            State::Faulted
        } else {
            State::Idle
        };
        self.fault.map_or(Transition::Stopped, Transition::Failed)
    }

    pub fn tick(
        &mut self,
        link: Option<&InputLink>,
        context: Context,
        acquire: impl FnOnce(Instant) -> Result<C, &'static str>,
    ) -> Transition {
        if matches!(self.state, State::Releasing(_)) {
            return self.release();
        }
        if matches!(self.state, State::Faulted) {
            return Transition::Unchanged;
        }
        let requested = link.is_some_and(InputLink::capture_requested);
        let transport = link.is_some_and(|link| {
            if self.started().is_some() {
                link.epoch() != 0
            } else {
                link.ready()
            }
        });
        let fault = if !context.session_active {
            Some("Capture revoked by session change or sleep")
        } else if !context.permitted {
            Some("Input permission revoked")
        } else if context.secure_input {
            Some("Capture revoked by Secure Input")
        } else {
            None
        };
        if !requested || !transport || fault.is_some() {
            if self.owns_cursor() {
                self.fault = fault;
                if let Some(link) = link {
                    link.end();
                }
                self.stop();
                return self.release();
            }
            return Transition::Unchanged;
        }
        let state = std::mem::replace(&mut self.state, State::Idle);
        match state {
            State::Idle => match acquire(context.now) {
                Ok(cursor) => self.state = State::Preparing(cursor),
                Err(error) => {
                    self.state = State::Faulted;
                    if let Some(link) = link {
                        link.end();
                    }
                    return Transition::Failed(error);
                }
            },
            State::Preparing(mut cursor) => match cursor.ready(context.ours, context.now) {
                Ok(true) if link.is_some_and(InputLink::begin) => {
                    self.state = State::Capturing {
                        cursor,
                        since: context.now,
                    };
                    return Transition::Started;
                }
                Ok(false) => self.state = State::Preparing(cursor),
                result => {
                    self.fault = result.err();
                    self.state = State::Releasing(cursor);
                    if let Some(link) = link {
                        link.end();
                    }
                    return self.release();
                }
            },
            State::Capturing { mut cursor, since } => {
                match cursor.ready(context.ours, context.now) {
                    Ok(true) => self.state = State::Capturing { cursor, since },
                    result => {
                        self.fault = Some(result.err().unwrap_or("Cursor ownership lost"));
                        self.state = State::Releasing(cursor);
                        if let Some(link) = link {
                            link.end();
                        }
                        return self.release();
                    }
                }
            }
            state => self.state = state,
        }
        Transition::Unchanged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        rc::Rc,
        sync::{Arc, atomic::AtomicU64},
        time::Duration,
    };

    struct Cursor {
        released: Rc<Cell<usize>>,
    }
    impl CursorLease for Cursor {
        fn ready(&mut self, ours: bool, _: Instant) -> Result<bool, &'static str> {
            Ok(ours)
        }
    }
    impl Drop for Cursor {
        fn drop(&mut self) {
            self.released.set(self.released.get() + 1);
        }
    }
    fn fixture() -> (InputLink, Session<Cursor>, Context, Rc<Cell<usize>>) {
        let (link, _) = InputLink::channel(Arc::new(AtomicU64::new(1)));
        link.set_profile_available(true);
        assert!(link.request_profile());
        link.set_ready(1);
        (
            link,
            Session::default(),
            Context {
                now: Instant::now(),
                ours: true,
                permitted: true,
                session_active: true,
                secure_input: false,
            },
            Rc::new(Cell::new(0)),
        )
    }
    #[test]
    fn acquisition_failure_revokes_request_without_owning_a_cursor() {
        let (link, mut session, cx, released) = fixture();
        assert_eq!(
            session.tick(Some(&link), cx, |_| Err("activation failed")),
            Transition::Failed("activation failed")
        );
        assert!(!session.owns_cursor());
        assert!(!link.capture_requested());
        assert_eq!(released.get(), 0);
        assert_eq!(
            session.tick(Some(&link), cx, |_| panic!("fault must not silently retry")),
            Transition::Unchanged
        );
    }
    #[test]
    fn active_capture_survives_ticks_and_release_is_deferred_and_exactly_once() {
        let (link, mut session, mut cx, released) = fixture();
        assert_eq!(
            session.tick(Some(&link), cx, |_| Ok(Cursor {
                released: released.clone()
            })),
            Transition::Unchanged
        );
        assert_eq!(
            session.tick(Some(&link), cx, |_| panic!("already acquired")),
            Transition::Started
        );
        assert!(!link.ready());
        for _ in 0..100 {
            cx.now += Duration::from_millis(5);
            assert_eq!(
                session.tick(Some(&link), cx, |_| panic!("already acquired")),
                Transition::Unchanged
            );
            assert!(session.started().is_some());
        }
        session.stop();
        session.stop();
        assert_eq!(
            released.get(),
            0,
            "callback must not release native ownership"
        );
        assert_eq!(session.release(), Transition::Stopped);
        assert_eq!(session.release(), Transition::Unchanged);
        assert_eq!(released.get(), 1);
    }
    #[test]
    fn cancellation_and_faults_release_preparing_and_active_sessions() {
        for active in [false, true] {
            for cause in 0..6 {
                let (link, mut session, mut cx, released) = fixture();
                session.tick(Some(&link), cx, |_| {
                    Ok(Cursor {
                        released: released.clone(),
                    })
                });
                if active {
                    session.tick(Some(&link), cx, |_| unreachable!());
                }
                match cause {
                    0 => link.end(),
                    1 => link.disconnect(),
                    2 => cx.permitted = false,
                    3 => cx.session_active = false,
                    4 => cx.secure_input = true,
                    _ => link.fail("queue full"),
                }
                let transition = session.tick(Some(&link), cx, |_| panic!("must not reacquire"));
                assert!(matches!(
                    transition,
                    Transition::Stopped | Transition::Failed(_)
                ));
                assert_eq!(released.get(), 1);
                assert!(session.started().is_none());
                assert_eq!(link.epoch(), 0);
            }
        }
    }
    #[test]
    fn activation_waits_without_consuming_input_and_lost_focus_revokes_capture() {
        let (link, mut session, mut cx, released) = fixture();
        cx.ours = false;
        session.tick(Some(&link), cx, |_| {
            Ok(Cursor {
                released: released.clone(),
            })
        });
        session.tick(Some(&link), cx, |_| unreachable!());
        assert!(session.started().is_none());
        assert_eq!(link.epoch(), 0);
        cx.ours = true;
        assert_eq!(
            session.tick(Some(&link), cx, |_| unreachable!()),
            Transition::Started
        );
        cx.ours = false;
        assert!(matches!(
            session.tick(Some(&link), cx, |_| unreachable!()),
            Transition::Failed(_)
        ));
        assert_eq!(released.get(), 1);
    }
}
