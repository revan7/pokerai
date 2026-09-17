use super::*;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

struct RecordingPort {
    calls: Arc<Mutex<Vec<String>>>,
}
impl service::EnginePort for RecordingPort {
    fn dispatch(&mut self, op: service::Op) -> Result<Value, error::AppError> {
        self.calls.lock().unwrap().push(op.name().into());
        match op {
            service::Op::PresolverStatus => Ok(json!({"paused":false,"done":4})),
            service::Op::Cancel(_) | service::Op::Finish | service::Op::Abandon | service::Op::Pause | service::Op::Resume => {
                Ok(Value::Null)
            }
            _ => Err(error::AppError::Engine { message: "script has no reply".into() }),
        }
    }
    fn shutdown(&mut self) {
        self.calls.lock().unwrap().push("shutdown".into());
    }
}

#[test]
fn service_dispatches_by_name_and_reports_engine_errors() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let service = service::Service::spawn(Box::new(RecordingPort { calls: calls.clone() }));
    let status: Value = tauri::async_runtime::block_on(service.request(service::Op::PresolverStatus)).unwrap();
    assert_eq!(status["done"], 4);
    tauri::async_runtime::block_on(service.request::<()>(service::Op::Finish)).unwrap();
    tauri::async_runtime::block_on(service.request::<()>(service::Op::Cancel(9))).unwrap();
    let error = tauri::async_runtime::block_on(service.request::<Value>(service::Op::Undo)).unwrap_err();
    assert!(matches!(error, error::AppError::Engine { .. }));
    assert_eq!(*calls.lock().unwrap(), vec!["presolver_status", "finish_hand", "cancel", "undo"]);
    service.stop();
}

struct HeldPort {
    started: Option<std::sync::mpsc::Sender<()>>,
    gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
}
impl service::EnginePort for HeldPort {
    fn dispatch(&mut self, _: service::Op) -> Result<Value, error::AppError> {
        if let Some(tx) = self.started.take() {
            tx.send(()).unwrap();
            let (lock, cv) = &*self.gate;
            let ready = lock.lock().unwrap();
            drop(cv.wait_while(ready, |ready| !*ready).unwrap());
        }
        Ok(Value::Null)
    }
    fn shutdown(&mut self) {}
}

#[test]
fn waiting_engine_does_not_block_command_poll_and_queue_saturates() {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
        time::Duration,
    };
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let (tx, rx) = std::sync::mpsc::channel();
    let service = service::Service::spawn(Box::new(HeldPort { started: Some(tx), gate: gate.clone() }));
    let mut cx = Context::from_waker(Waker::noop());
    let mut first = Box::pin(service.request::<Value>(service::Op::PresolverStatus));
    assert!(matches!(first.as_mut().poll(&mut cx), Poll::Pending));
    rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let mut queued = Vec::new();
    for _ in 0..32 {
        let mut f = Box::pin(service.request::<Value>(service::Op::PresolverStatus));
        assert!(f.as_mut().poll(&mut cx).is_pending());
        queued.push(f);
    }
    let mut overflow = Box::pin(service.request::<Value>(service::Op::PresolverStatus));
    assert!(matches!(overflow.as_mut().poll(&mut cx), Poll::Ready(Err(error::AppError::Busy))));
    *gate.0.lock().unwrap() = true;
    gate.1.notify_one();
    tauri::async_runtime::block_on(first).unwrap();
    for f in queued {
        tauri::async_runtime::block_on(f).unwrap();
    }
    service.stop();
}

#[test]
fn stop_is_idempotent_and_delegates_once() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let s = service::Service::spawn(Box::new(RecordingPort { calls: calls.clone() }));
    s.stop();
    s.stop();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while !s.stopped() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(s.stopped());
    assert_eq!(calls.lock().unwrap().iter().filter(|x| *x == "shutdown").count(), 1);
}
