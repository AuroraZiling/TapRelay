//! Admission to physical capture. Native observers belong to the input run
//! loop, not the foreground application. The caller releases router owners
//! before closing InputLink admission on every stop/failure transition.
use std::time::Instant;
use taprelay_core::passthrough::InputLink;

#[derive(Default)]
pub(super) struct Session {
    since: Option<Instant>,
}

#[derive(Clone, Copy)]
pub(super) struct Context {
    pub now: Instant,
    pub permitted: bool,
    pub session_active: bool,
    pub secure_input: bool,
    pub input_error: Option<&'static str>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Transition {
    Unchanged,
    Started,
    Stopped,
    Failed(&'static str),
}

impl Session {
    pub fn started(&self) -> Option<Instant> {
        self.since
    }
    pub fn stop(&mut self) {
        self.since = None;
    }

    pub fn tick(&mut self, link: Option<&InputLink>, context: Context) -> Transition {
        let active = self.since.is_some();
        let requested = link.is_some_and(InputLink::capture_requested);
        let transport = link.is_some_and(|link| {
            if active {
                link.epoch() != 0
            } else {
                link.ready()
            }
        });
        if !requested || !transport {
            return if self.since.take().is_some() {
                Transition::Stopped
            } else {
                Transition::Unchanged
            };
        }
        let error = if !context.session_active {
            Some("Capture revoked by session change or sleep")
        } else if !context.permitted {
            Some("Input permission revoked")
        } else if context.secure_input {
            Some("Capture revoked by Secure Input")
        } else {
            context.input_error
        };
        if let Some(error) = error {
            self.stop();
            return Transition::Failed(error);
        }
        if !active && link.is_some_and(InputLink::begin) {
            self.since = Some(context.now);
            return Transition::Started;
        }
        Transition::Unchanged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicU64};

    fn fixture() -> (InputLink, Session, Context) {
        let (link, _) = InputLink::channel(Arc::new(AtomicU64::new(1)));
        link.set_profile_available(true);
        assert!(link.request_profile());
        link.set_ready(1);
        (
            link,
            Session::default(),
            Context {
                now: Instant::now(),
                permitted: true,
                session_active: true,
                secure_input: false,
                input_error: None,
            },
        )
    }

    #[test]
    fn starts_without_foreground_ownership_and_can_restart_after_exit() {
        let (link, mut session, cx) = fixture();
        for _ in 0..3 {
            assert_eq!(session.tick(Some(&link), cx), Transition::Started);
            assert_eq!(session.started(), Some(cx.now));
            for _ in 0..100 {
                assert_eq!(session.tick(Some(&link), cx), Transition::Unchanged);
            }
            link.end();
            assert_eq!(session.tick(Some(&link), cx), Transition::Stopped);
            assert_eq!(session.tick(Some(&link), cx), Transition::Unchanged);
            assert!(link.request_profile());
            link.set_ready(1);
        }
    }

    #[test]
    fn faults_do_not_close_admission_before_router_owners_are_released() {
        for active in [false, true] {
            for cause in 0..4 {
                let (link, mut session, mut cx) = fixture();
                if active {
                    assert_eq!(session.tick(Some(&link), cx), Transition::Started);
                }
                match cause {
                    0 => cx.permitted = false,
                    1 => cx.session_active = false,
                    2 => cx.secure_input = true,
                    _ => cx.input_error = Some("No physical relative mouse"),
                }
                assert!(matches!(
                    session.tick(Some(&link), cx),
                    Transition::Failed(_)
                ));
                assert!(session.started().is_none());
                assert_eq!(
                    link.epoch() != 0,
                    active,
                    "caller must release owners first"
                );
                link.end();
                assert_eq!(session.tick(Some(&link), cx), Transition::Unchanged);
            }
        }
    }

    #[test]
    fn cancellation_and_transport_loss_never_restart_capture() {
        for active in [false, true] {
            for disconnect in [false, true] {
                let (link, mut session, cx) = fixture();
                if active {
                    session.tick(Some(&link), cx);
                }
                if disconnect {
                    link.disconnect();
                } else {
                    link.end();
                }
                let expected = if active {
                    Transition::Stopped
                } else {
                    Transition::Unchanged
                };
                assert_eq!(session.tick(Some(&link), cx), expected);
                assert!(session.started().is_none());
                link.set_ready(1);
                assert_eq!(session.tick(Some(&link), cx), Transition::Unchanged);
                assert_eq!(link.epoch(), 0);
            }
        }
    }
}
