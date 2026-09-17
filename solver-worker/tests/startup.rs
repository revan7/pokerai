mod common;
use common::Worker;
use std::time::Duration;

#[test]
fn ready_reports_features() {
    let mut w = Worker::spawn(4);
    let ready = w.recv(Duration::from_secs(5)).expect("ready within 5 s");
    assert_eq!(ready["type"], "ready");
    assert_eq!(ready["proto_version"], 3);
    assert_eq!(ready["adapter_version"], 1);
    assert_eq!(ready["threads"], 4);
    assert_eq!(ready["solver_commit"], "9d1509fe5077d019825f833eed04b16d342dfda1");
    assert!(ready["build_features"].as_array().unwrap().iter().any(|f| f == "avx2"));
    for cap in ["solve", "lock", "cancel", "street_export", "i16"] {
        assert!(ready["capabilities"].as_array().unwrap().iter().any(|c| c == cap), "missing capability {cap}");
    }
    // stdin EOF behaves like shutdown without the ack: exit 0 within 2 s
    w.close_stdin();
    assert_eq!(w.wait_exit(Duration::from_secs(2)), Some(0));
}
