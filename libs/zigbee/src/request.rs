use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use crossbeam_channel::{bounded, unbounded, Receiver, RecvTimeoutError, Sender};

use super::frame::{MessageType, ZnpFrame};
use super::transport::Transport;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const EVENT_CAPACITY: usize = 128;
const DISPATCH_POLL: Duration = Duration::from_millis(200);

type FrameSink = Box<dyn Fn(&ZnpFrame) -> Result<()> + Send + Sync>;

pub struct RequestEngine {
    send: FrameSink,
    routes: Arc<Mutex<Routes>>,
    sreq_turn: Mutex<()>,
    next_watch: AtomicU64,
    events_rx: Receiver<ZnpFrame>,
}

#[derive(Default)]
struct Routes {
    srsp: Option<Route>,
    areq: Vec<Route>,
}

struct Route {
    id: u64,
    subsystem: u8,
    cmd1: u8,
    tx: Sender<ZnpFrame>,
}

impl Route {
    fn matches(&self, frame: &ZnpFrame) -> bool {
        frame.subsystem() == self.subsystem && frame.cmd1 == self.cmd1
    }
}

pub struct AreqWatch<'a> {
    engine: &'a RequestEngine,
    id: u64,
    subsystem: u8,
    cmd1: u8,
    rx: Receiver<ZnpFrame>,
}

impl RequestEngine {
    pub fn new(transport: Transport) -> Self {
        let frames = transport.frames();
        Self::with_link(
            Box::new(move |frame: &ZnpFrame| transport.send(frame)),
            frames,
        )
    }

    fn with_link(send: FrameSink, frames: Receiver<ZnpFrame>) -> Self {
        let routes = Arc::new(Mutex::new(Routes::default()));
        let (events_tx, events_rx) = bounded(EVENT_CAPACITY);
        let dispatch_routes = Arc::downgrade(&routes);
        let spawned = thread::Builder::new()
            .name("znp-dispatch".into())
            .spawn(move || dispatch(&frames, &dispatch_routes, &events_tx));
        if let Err(error) = spawned {
            log::warn!("znp dispatcher thread failed to start: {error}");
        }
        Self {
            send,
            routes,
            sreq_turn: Mutex::new(()),
            next_watch: AtomicU64::new(0),
            events_rx,
        }
    }

    pub fn sreq(&self, subsystem: u8, cmd1: u8, data: Vec<u8>) -> Result<ZnpFrame> {
        self.sreq_timeout(subsystem, cmd1, data, DEFAULT_TIMEOUT)
    }

    pub fn sreq_timeout(
        &self,
        subsystem: u8,
        cmd1: u8,
        data: Vec<u8>,
        timeout: Duration,
    ) -> Result<ZnpFrame> {
        let _turn = self
            .sreq_turn
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (tx, rx) = bounded(1);
        let id = self.next_watch.fetch_add(1, Ordering::Relaxed);
        self.routes().srsp = Some(Route {
            id,
            subsystem,
            cmd1,
            tx,
        });
        let sent = (self.send)(&ZnpFrame::sreq(subsystem, cmd1, data));
        let response = sent.and_then(|()| {
            rx.recv_timeout(timeout).map_err(|_| {
                anyhow!(
                    "SRSP timeout: subsystem=0x{:02X} cmd=0x{:02X}",
                    subsystem,
                    cmd1
                )
            })
        });
        let mut routes = self.routes();
        if routes.srsp.as_ref().is_some_and(|route| route.id == id) {
            routes.srsp = None;
        }
        response
    }

    pub fn areq(&self, subsystem: u8, cmd1: u8, data: Vec<u8>) -> Result<()> {
        (self.send)(&ZnpFrame::areq(subsystem, cmd1, data))
    }

    pub fn events(&self) -> &Receiver<ZnpFrame> {
        &self.events_rx
    }

    pub fn watch_areq(&self, subsystem: u8, cmd1: u8) -> AreqWatch<'_> {
        let (tx, rx) = unbounded();
        let id = self.next_watch.fetch_add(1, Ordering::Relaxed);
        self.routes().areq.push(Route {
            id,
            subsystem,
            cmd1,
            tx,
        });
        AreqWatch {
            engine: self,
            id,
            subsystem,
            cmd1,
            rx,
        }
    }

    fn routes(&self) -> MutexGuard<'_, Routes> {
        self.routes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl AreqWatch<'_> {
    pub fn wait_matching(
        &self,
        timeout: Duration,
        matches: impl Fn(&ZnpFrame) -> bool,
    ) -> Result<ZnpFrame> {
        let deadline = Instant::now() + timeout;
        loop {
            let frame = self.wait(deadline.saturating_duration_since(Instant::now()))?;
            if matches(&frame) {
                return Ok(frame);
            }
        }
    }

    pub fn wait(&self, timeout: Duration) -> Result<ZnpFrame> {
        self.rx.recv_timeout(timeout).map_err(|_| {
            anyhow!(
                "AREQ timeout: subsystem=0x{:02X} cmd=0x{:02X}",
                self.subsystem,
                self.cmd1
            )
        })
    }
}

impl Drop for AreqWatch<'_> {
    fn drop(&mut self) {
        self.engine
            .routes()
            .areq
            .retain(|route| route.id != self.id);
    }
}

fn dispatch(frames: &Receiver<ZnpFrame>, routes: &Weak<Mutex<Routes>>, events: &Sender<ZnpFrame>) {
    loop {
        match frames.recv_timeout(DISPATCH_POLL) {
            Ok(frame) => {
                let Some(routes) = routes.upgrade() else {
                    return;
                };
                route(frame, &routes, events);
            }
            Err(RecvTimeoutError::Timeout) if routes.strong_count() > 0 => {}
            Err(_) => return,
        }
    }
}

fn route(frame: ZnpFrame, routes: &Mutex<Routes>, events: &Sender<ZnpFrame>) {
    let mut routes = routes.lock().unwrap_or_else(PoisonError::into_inner);
    match frame.message_type() {
        MessageType::Srsp => match routes.srsp.take() {
            Some(route) if route.matches(&frame) => {
                let _ = route.tx.try_send(frame);
            }
            waiting => {
                routes.srsp = waiting;
                log::debug!(
                    "unexpected SRSP: subsystem=0x{:02X} cmd=0x{:02X}",
                    frame.subsystem(),
                    frame.cmd1
                );
            }
        },
        MessageType::Areq => {
            let mut claimed = false;
            for route in routes.areq.iter().filter(|route| route.matches(&frame)) {
                claimed |= route.tx.send(frame.clone()).is_ok();
            }
            if !claimed && events.try_send(frame).is_err() {
                log::warn!("znp event queue full, dropping an AREQ");
            }
        }
        MessageType::Sreq => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::build_cmd0;
    use crate::subsystem::{sys, zdo, AF, SYS, ZDO};

    const WAIT: Duration = Duration::from_secs(2);

    fn srsp(subsystem: u8, cmd1: u8, data: Vec<u8>) -> ZnpFrame {
        ZnpFrame {
            cmd0: build_cmd0(MessageType::Srsp, subsystem),
            cmd1,
            data,
        }
    }

    fn engine_with(
        reply: impl Fn(&ZnpFrame) -> Vec<ZnpFrame> + Send + Sync + 'static,
    ) -> (RequestEngine, Sender<ZnpFrame>) {
        let (device_tx, frames) = unbounded();
        let replies = device_tx.clone();
        let engine = RequestEngine::with_link(
            Box::new(move |frame: &ZnpFrame| {
                for answer in reply(frame) {
                    replies.send(answer)?;
                }
                Ok(())
            }),
            frames,
        );
        (engine, device_tx)
    }

    #[test]
    fn an_announce_while_idle_reaches_the_event_loop() {
        let (engine, device) = engine_with(|_| Vec::new());
        let announce = ZnpFrame::areq(ZDO, zdo::END_DEVICE_ANNCE_IND, vec![0; 12]);

        device.send(announce.clone()).unwrap();

        assert_eq!(engine.events().recv_timeout(WAIT).unwrap(), announce);
    }

    #[test]
    fn a_request_gets_its_response_past_unrelated_frames() {
        let (engine, _device) = engine_with(|frame| {
            vec![
                ZnpFrame::areq(ZDO, zdo::END_DEVICE_ANNCE_IND, vec![1]),
                srsp(AF, frame.cmd1, vec![0xEE]),
                srsp(frame.subsystem(), frame.cmd1, vec![0x00]),
            ]
        });

        let response = engine.sreq(SYS, sys::PING, vec![]).unwrap();

        assert_eq!(response.data, vec![0x00]);
        assert_eq!(
            engine.events().recv_timeout(WAIT).unwrap().cmd1,
            zdo::END_DEVICE_ANNCE_IND
        );
    }

    #[test]
    fn an_answer_that_arrives_before_the_wait_is_kept_for_the_watch() {
        let (engine, _device) = engine_with(|frame| {
            vec![
                srsp(frame.subsystem(), frame.cmd1, vec![0x00]),
                ZnpFrame::areq(ZDO, zdo::ACTIVE_EP_RSP, vec![7]),
            ]
        });

        let watch = engine.watch_areq(ZDO, zdo::ACTIVE_EP_RSP);
        engine.sreq(ZDO, zdo::ACTIVE_EP_REQ, vec![]).unwrap();
        std::thread::sleep(Duration::from_millis(50));

        assert_eq!(watch.wait(WAIT).unwrap().data, vec![7]);
        assert!(
            engine.events().try_recv().is_err(),
            "a watched frame is not an event"
        );
    }

    #[test]
    fn requests_from_several_threads_each_get_their_own_response() {
        let (engine, _device) =
            engine_with(|frame| vec![srsp(frame.subsystem(), frame.cmd1, frame.data.clone())]);

        std::thread::scope(|scope| {
            for value in 0..8u8 {
                let engine = &engine;
                scope.spawn(move || {
                    let response = engine.sreq(SYS, sys::OSAL_NV_WRITE, vec![value]).unwrap();
                    assert_eq!(response.data, vec![value]);
                });
            }
        });
    }

    #[test]
    fn a_matching_wait_keeps_one_deadline_past_other_answers() {
        let (engine, device) = engine_with(|_| Vec::new());
        let watch = engine.watch_areq(ZDO, zdo::ACTIVE_EP_RSP);
        for _ in 0..3 {
            device
                .send(ZnpFrame::areq(ZDO, zdo::ACTIVE_EP_RSP, vec![1]))
                .unwrap();
        }
        device
            .send(ZnpFrame::areq(ZDO, zdo::ACTIVE_EP_RSP, vec![2]))
            .unwrap();

        let started = Instant::now();
        assert_eq!(
            watch
                .wait_matching(WAIT, |frame| frame.data == [2])
                .unwrap()
                .data,
            vec![2]
        );
        let error = watch
            .wait_matching(Duration::from_millis(100), |_| false)
            .unwrap_err();
        assert!(error.to_string().contains("AREQ timeout"), "{error}");
        assert!(started.elapsed() < WAIT);
    }

    #[test]
    fn a_dropped_watch_lets_the_frame_become_an_event() {
        let (engine, device) = engine_with(|_| Vec::new());
        drop(engine.watch_areq(ZDO, zdo::STATE_CHANGE_IND));
        let state = ZnpFrame::areq(ZDO, zdo::STATE_CHANGE_IND, vec![0x09]);

        device.send(state.clone()).unwrap();

        assert_eq!(engine.events().recv_timeout(WAIT).unwrap(), state);
    }

    #[test]
    fn a_missing_response_times_out() {
        let (engine, _device) = engine_with(|_| Vec::new());

        let error = engine
            .sreq_timeout(SYS, sys::PING, vec![], Duration::from_millis(50))
            .unwrap_err();

        assert!(error.to_string().contains("SRSP timeout"), "{error}");
    }
}
