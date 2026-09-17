use super::*;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tauri::{
    ipc::{CallbackFn, InvokeBody, InvokeResponseBody},
    test::{get_ipc_response, mock_builder, mock_context, noop_assets},
    webview::InvokeRequest,
};

fn invoke(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    args: Value,
) -> Result<Value, Value> {
    get_ipc_response(
        window,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.into(),
        },
    )
    .map(|body| body.deserialize::<Value>().unwrap())
}

#[test]
fn commands_delegate_and_tag_is_unsupported() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let service = service::Service::spawn(Box::new(RecordingPort { calls: calls.clone() }));
    let app = configure(mock_builder(), service).build(mock_context(noop_assets())).unwrap();
    let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    assert_eq!(invoke(&w, "presolver_status", json!({})).unwrap()["done"], 4);
    for command in ["finish_hand", "abandon_hand", "presolver_pause", "presolver_resume"] {
        assert_eq!(invoke(&w, command, json!({})).unwrap(), Value::Null);
    }
    assert_eq!(invoke(&w, "cancel", json!({"decision_id":9})).unwrap(), Value::Null);
    let error = invoke(&w, "set_seat_tag", json!({"seat":2,"tag":"unknown"})).unwrap_err();
    assert!(error.to_string().contains("phase 1"));
    // Complete ordered dispatch list: `{}` for every no-argument operation, the
    // decoded `decision_id` for `cancel`, and no `set_seat_tag` entry at all
    // (the phase-1 stub never reaches the engine). This fails if any two
    // operations are swapped, if `cancel`'s id is substituted, or if
    // `set_seat_tag` ever dispatches.
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            ("presolver_status".to_string(), json!({})),
            ("finish_hand".to_string(), json!({})),
            ("abandon_hand".to_string(), json!({})),
            ("presolver_pause".to_string(), json!({})),
            ("presolver_resume".to_string(), json!({})),
            ("cancel".to_string(), json!({"decision_id":9})),
        ]
    );
}

// tests.rs additions: scripted values are transport fixtures, never a rules implementation.
fn sample_hand() -> proto::HandState {
    serde_json::from_value(json!({
        "hand_id":10,"hand_revision":7,
        "config":{"config_revision":1,"chip_label":"$1","sb_chips":5,"bb_chips":10,
            "straddle":null,"rake":{"kind":"time_charge"}},
        "phase":{"phase":"betting","street":"preflop"},"button":0,"hero":0,
        "hero_cards":["As","Kd"],"dealt":[0,1,2,3,4,5],"stacks_start":vec![1000;6],
        "board":[],"actions":[],
        "derived":{"street":"preflop","to_act":3,"pot":15,
            "committed_this_street":[0,5,10,0,0,0],"stacks_remaining":[1000,995,990,1000,1000,1000],
            "folded":vec![false;6],"all_in":vec![false;6],"facing":10,"last_full_raise":10,"pots":[],
            "legal":[{"kind":"fold"},{"kind":"call","cost":10}]}
    })).unwrap()
}
fn sample_id() -> proto::DecisionIdentity {
    proto::DecisionIdentity { hand_id: 10, hand_revision: 7, decision_id: 90, config_revision: 1, model_revision: 0 }
}
type Recorded = Arc<Mutex<Vec<(String, Value)>>>;
struct ScriptPort { calls: Recorded, hand: proto::HandState, events: Vec<proto::RecommendationEvent> }
impl service::EnginePort for ScriptPort {
    fn dispatch(&mut self, op: service::Op) -> Result<Value, error::AppError> {
        use service::Op;
        let name = op.name().to_owned();
        let (args, reply) = match op {
            Op::Config(c) => (json!({"config":c}), serde_json::to_value(&c)?),
            Op::Begin(b) => (json!({"begin":b}), serde_json::to_value(&self.hand)?),
            Op::Hero(cards) => (json!({"cards":cards}), serde_json::to_value(&self.hand)?),
            Op::Board(board) => (json!({"board":board}), serde_json::to_value(&self.hand)?),
            Op::Action(action) => {
                if matches!(action, proto::Action::Raise { to: 0 }) {
                    return Err(error::AppError::Engine { message: "illegal raise".into() });
                }
                (json!({"action":action}), serde_json::to_value(&self.hand)?)
            },
            Op::Undo => (json!({}), serde_json::to_value(&self.hand)?),
            Op::Recommend(channel) => {
                let channel_id = channel.id();
                for e in &self.events { channel.send(e.clone()).unwrap(); }
                (json!({"on_event_channel_id":channel_id}), serde_json::to_value(sample_id())?)
            },
            Op::Cancel(id) => (json!({"decision_id":id}), Value::Null),
            Op::PresolverStatus => (json!({}), json!({"paused":false})),
            Op::Finish | Op::Abandon | Op::Pause | Op::Resume => (json!({}), Value::Null),
        };
        self.calls.lock().unwrap().push((name, args));
        Ok(reply)
    }
    fn shutdown(&mut self) {}
}
#[test]
fn all_state_command_arguments_and_engine_errors_cross_ipc() {
    let calls = Arc::new(Mutex::new(vec![]));
    let service = service::Service::spawn(Box::new(ScriptPort { calls: calls.clone(), hand: sample_hand(), events: vec![] }));
    let app = configure(mock_builder(), service.clone()).build(mock_context(noop_assets())).unwrap();
    let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    let c = sample_hand().config;
    let game = json!({"config_revision":c.config_revision,"chip_label":c.chip_label,"sb_chips":5,"bb_chips":10,
        "straddle":null,"rake":c.rake,"seats":[{"seat":0,"tag":null,"facts":[]},{"seat":1,"tag":null,"facts":[]},
        {"seat":2,"tag":null,"facts":[]}],"solver":{"threads":16,"target_bp":50,"flop_budget_s":10}});
    let rows = [
        ("set_game_config", json!({"config":game})),
        ("begin_hand", json!({"begin":{"button":0,"hero":0,"dealt":[0,1,2,3,4,5],
            "stacks":vec![1000;6],"hero_cards":null}})),
        ("set_hero_cards", json!({"cards":["As","Kd"]})),
        ("apply_action", json!({"action":{"kind":"raise","to":25}})),
        ("set_board", json!({"board":["Kh","7d","2c"]})), ("undo", json!({})),
    ];
    for (name, args) in &rows { assert!(invoke(&w, name, args.clone()).is_ok(), "{name}"); }
    assert_eq!(*calls.lock().unwrap(), rows.iter().map(|(n, a)| (n.to_string(), a.clone())).collect::<Vec<_>>());
    let before = calls.lock().unwrap().len();
    assert!(invoke(&w, "set_hero_cards", json!({"cards":["Zz","Kd"]})).is_err());
    assert_eq!(calls.lock().unwrap().len(), before); // serde rejects before engine dispatch.
    assert!(invoke(&w, "apply_action", json!({"action":{"kind":"raise","to":0}})).unwrap_err().to_string().contains("illegal raise"));
    assert_eq!(invoke(&w, "undo", json!({})).unwrap(), serde_json::to_value(sample_hand()).unwrap());
    service.stop();
}

#[test]
fn recommend_delivers_events_through_the_named_channel() {
    let calls = Arc::new(Mutex::new(vec![]));
    let event = proto::RecommendationEvent::NoDecision { identity: sample_id(), reason: "queued".into() };
    let service = service::Service::spawn(Box::new(ScriptPort {
        calls: calls.clone(),
        hand: sample_hand(),
        events: vec![event.clone()],
    }));
    let intercepted: Arc<Mutex<Vec<(u32, usize, Value)>>> = Arc::new(Mutex::new(vec![]));
    let intercepted_clone = intercepted.clone();
    let builder = mock_builder().channel_interceptor(move |_webview, callback_fn, index, body| {
        let payload = match body {
            InvokeResponseBody::Json(s) => serde_json::from_str(s).unwrap(),
            InvokeResponseBody::Raw(_) => Value::Null,
        };
        intercepted_clone.lock().unwrap().push((callback_fn.0, index, payload));
        true // consumed: MockRuntime has no real webview to eval a script into.
    });
    let app = configure(builder, service.clone()).build(mock_context(noop_assets())).unwrap();
    let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();

    let response = invoke(&w, "recommend", json!({"on_event":"__CHANNEL__:42"})).unwrap();
    assert_eq!(response, serde_json::to_value(sample_id()).unwrap());
    assert_eq!(*calls.lock().unwrap(), vec![("recommend".to_string(), json!({"on_event_channel_id":42}))]);

    let events = intercepted.lock().unwrap();
    assert_eq!(events.len(), 1, "exactly one scripted event should reach the channel");
    assert_eq!(events[0].0, 42, "the event must reach the channel id supplied by on_event");
    assert_eq!(events[0].2, serde_json::to_value(&event).unwrap());
    drop(events);

    let before = calls.lock().unwrap().len();
    assert!(invoke(&w, "recommend", json!({})).is_err(), "a missing on_event must be rejected");
    assert!(invoke(&w, "recommend", json!({"onEvent":"__CHANNEL__:42"})).is_err(), "camelCase onEvent must be rejected (rename_all = snake_case)");
    assert!(invoke(&w, "recommend", json!({"on_event":"not-a-channel"})).is_err(), "a malformed channel value must be rejected");
    assert_eq!(calls.lock().unwrap().len(), before, "rejected arguments must never reach engine dispatch");

    service.stop();
}

struct RecordingPort {
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}
impl service::EnginePort for RecordingPort {
    fn dispatch(&mut self, op: service::Op) -> Result<Value, error::AppError> {
        use service::Op;
        let name = op.name().to_owned();
        let (args, reply) = match op {
            Op::Cancel(id) => (json!({"decision_id": id}), Ok(Value::Null)),
            Op::PresolverStatus => (json!({}), Ok(json!({"paused":false,"done":4}))),
            Op::Finish | Op::Abandon | Op::Pause | Op::Resume => (json!({}), Ok(Value::Null)),
            _ => (json!({}), Err(error::AppError::Engine { message: "script has no reply".into() })),
        };
        self.calls.lock().unwrap().push((name, args));
        reply
    }
    fn shutdown(&mut self) {
        self.calls.lock().unwrap().push(("shutdown".to_string(), json!({})));
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
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            ("presolver_status".to_string(), json!({})),
            ("finish_hand".to_string(), json!({})),
            ("cancel".to_string(), json!({"decision_id":9})),
            ("undo".to_string(), json!({})),
        ]
    );
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
    assert_eq!(calls.lock().unwrap().iter().filter(|(name, _)| name == "shutdown").count(), 1);
}
