use crate::error::AppError;
use proto::*;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use tauri::ipc::Channel;

pub enum Op {
    Config(GameConfig),
    Begin(BeginHand),
    Hero([Card; 2]),
    Action(Action),
    Board(Vec<Card>),
    Undo,
    Recommend(Channel<RecommendationEvent>),
    Cancel(u64),
    Finish,
    Abandon,
    PresolverStatus,
    Pause,
    Resume,
}

impl Op {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Config(_) => "set_game_config",
            Self::Begin(_) => "begin_hand",
            Self::Hero(_) => "set_hero_cards",
            Self::Action(_) => "apply_action",
            Self::Board(_) => "set_board",
            Self::Undo => "undo",
            Self::Recommend(_) => "recommend",
            Self::Cancel(_) => "cancel",
            Self::Finish => "finish_hand",
            Self::Abandon => "abandon_hand",
            Self::PresolverStatus => "presolver_status",
            Self::Pause => "presolver_pause",
            Self::Resume => "presolver_resume",
        }
    }
}

pub trait EnginePort: Send + 'static {
    fn dispatch(&mut self, op: Op) -> Result<Value, AppError>;
    fn shutdown(&mut self);
}

type Reply = futures_channel::oneshot::Sender<Result<Value, AppError>>;

#[derive(Clone)]
pub struct Service {
    tx: mpsc::SyncSender<(Op, Reply)>,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}

impl Service {
    pub fn spawn(mut port: Box<dyn EnginePort>) -> Self {
        let (tx, rx) = mpsc::sync_channel::<(Op, Reply)>(32);
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let (s, d) = (stop.clone(), done.clone());
        std::thread::Builder::new()
            .name("app-dispatch".into())
            .spawn(move || {
                while !s.load(Ordering::Acquire) {
                    match rx.recv_timeout(std::time::Duration::from_millis(20)) {
                        Ok((op, reply)) => {
                            let _ = reply.send(port.dispatch(op));
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                while let Ok((_, reply)) = rx.try_recv() {
                    let _ = reply.send(Err(AppError::Closed));
                }
                port.shutdown();
                d.store(true, Ordering::Release);
            })
            .expect("app-dispatch thread");
        Self { tx, stop, done }
    }

    pub async fn request<T: DeserializeOwned>(&self, op: Op) -> Result<T, AppError> {
        if self.stop.load(Ordering::Acquire) {
            return Err(AppError::Closed);
        }
        let (tx, rx) = futures_channel::oneshot::channel();
        self.tx.try_send((op, tx)).map_err(|e| match e {
            mpsc::TrySendError::Full(_) => AppError::Busy,
            mpsc::TrySendError::Disconnected(_) => AppError::Closed,
        })?;
        Ok(serde_json::from_value(rx.await.map_err(|_| AppError::Closed)??)?)
    }

    /// Nonblocking: sets the reserved shutdown signal and returns immediately, it does not
    /// wait for the dispatch thread to notice it or for `port.shutdown()` to run. A full
    /// queue cannot prevent signalling shutdown, since this never touches `tx`. The dispatch
    /// loop's `recv_timeout(20ms)` bounds only its *idle* wait for the next queued command —
    /// it does not bound, and cannot interrupt, a `port.dispatch(op)` call already in
    /// progress; a stuck engine call delays `stopped()` becoming true until it returns.
    /// Once the loop does observe the signal, any commands still queued at that point are
    /// rejected with `AppError::Closed` rather than dispatched.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    /// True only after the dispatch thread has observed `stop`, drained the queue with
    /// `AppError::Closed` replies, and called `port.shutdown()`. See `stop`'s doc comment for
    /// the exact, non-instantaneous timing this implies.
    pub fn stopped(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
}
