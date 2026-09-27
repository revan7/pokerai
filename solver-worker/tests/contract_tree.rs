mod common;
use common::{fixture_lines, Worker};
use proto::{combo_cards, combo_index, Card, Range1326};
use serde_json::{json, Value};
use std::time::Duration;

const S: Duration = Duration::from_secs(1);
fn edit(line: &str, f: impl FnOnce(&mut Value)) -> String { let mut v: Value = serde_json::from_str(line).unwrap(); f(&mut v); v.to_string() }
fn ready(w: &Worker) { assert_eq!(w.recv(5 * S).unwrap()["type"], "ready"); }
fn result_of(w: &Worker, id: &str) -> Value { w.recv_until(60 * S, |m| m["type"] == "result" && m["id"] == id).unwrap_or_else(|| panic!("no result {id}")) }
fn cases() -> Vec<Value> { fixture_lines("materialization_cases").iter().map(|l| serde_json::from_str(l).unwrap()).collect() }
fn case(name: &str) -> Value { cases().into_iter().find(|c| c["case"] == name).unwrap() }

#[test]
fn tree_materialization_matches_library() {
    // equality is checked in-process for all 47 cases (Task 8); here a REALIZED tree that differs from `tree.materialized`
    // is result{error{tree_mismatch}} on the wire. A later-street None donk never reaches this path: precheck rejects it.
    let mut w = Worker::spawn(4); ready(&w);
    let river = &fixture_lines("river_two_combo")[0];
    w.send(&edit(river, |v| { v["id"] = json!("100"); v["tree"]["materialized"][2]["terminal_pots"][1] = json!(299); }));
    assert_eq!(result_of(&w, "100")["error"]["code"], "tree_mismatch");
    w.send(&edit(river, |v| { v["id"] = json!("101"); v["tree"]["materialized"][1]["terminal_pots"][0] = Value::Null; }));   // missing terminal marker
    assert_eq!(result_of(&w, "101")["error"]["code"], "tree_mismatch");
    let flop = &fixture_lines("flop_cancel")[0];
    let full = case("facing_350_full");
    w.send(&edit(flop, |v| {                                                                       // per-street reset menu at the turn root of facing_test_v1
        v["id"] = json!("102"); v["pot"] = json!(100); v["stack_oop"] = json!(350); v["stack_ip"] = json!(350); v["tree"] = full["tree"].clone();
        let turn = v["tree"]["materialized"].as_array().unwrap().iter().position(|n| n["path"] == json!([1, 1])).unwrap();
        v["tree"]["materialized"][turn]["actions"] = json!([{"kind": "check"}, {"kind": "bet", "to": 100}]);
        v["tree"]["materialized"][turn]["terminal_pots"] = json!([null, null]);
    }));
    assert_eq!(result_of(&w, "102")["error"]["code"], "tree_mismatch");
    // `deadline_ms`/`extraction_margin_ms` here are a liveness bound, not a timing assertion (as elsewhere
    // in this suite): 400/200 (the brief's own figures) leaves under 200 ms of Solving budget once Building
    // this 3-street, 78-node `facing_350_full` tree has run, and Building alone measured 130-140 ms here,
    // consistently returning `error{no_iteration}` rather than `ok`/`best_so_far` -- not what 102's mismatch
    // contrasts against. 3000/600 measured >= 3 real iterations well inside budget on this machine.
    w.send(&edit(flop, |v| { v["id"] = json!("103"); v["pot"] = json!(100); v["stack_oop"] = json!(350); v["stack_ip"] = json!(350); v["tree"] = full["tree"].clone(); v["deadline_ms"] = json!(3000); v["extraction_margin_ms"] = json!(600); }));
    let ok = result_of(&w, "103");
    assert!(ok["status"] == "ok" || ok["status"] == "best_so_far", "{ok}");                        // the unaltered facing_350 tree is accepted
    // §13.2: the ROOT street's own `None` donk is legal and never a tree_mismatch. `facing_test_v1` is flop-rooted,
    // so `menus.flop.donk` is already null here; setting it explicitly must change nothing.
    w.send(&edit(flop, |v| { v["id"] = json!("104"); v["pot"] = json!(100); v["stack_oop"] = json!(350); v["stack_ip"] = json!(350); v["tree"] = full["tree"].clone(); v["tree"]["menus"]["flop"]["donk"] = Value::Null; v["deadline_ms"] = json!(3000); v["extraction_margin_ms"] = json!(600); }));
    let root_none = result_of(&w, "104");
    assert!(root_none["status"] == "ok" || root_none["status"] == "best_so_far", "{root_none}");
    // A later street's `None` is rejected before any work by `precheck` (Task 13's `protocol_rejections`),
    // so it can never appear here as a `tree_mismatch`.
}

#[test]
fn wager_cap_remove_lines() {
    use postflop_solver::Action as L;
    use solver_worker::{history::history_to_lib, tree_build::build};
    // `cap3_three_wagers` (turn_std_v1, pot 100 / eff 1000): unlike `cap1_two_wagers`, the raw (uncapped)
    // tree never offers an all-in at the node facing OOP's third wager -- verified by rebuilding the same
    // case with `wager_cap` widened to 255 (no removal), which leaves `[Fold, Call, Raise(520)]`, no
    // `AllIn`. So the cap here has only a `Raise` to remove; the fixture's own numbers, not a loosened
    // assertion.
    for (name, expected_after_prefix) in [("cap1_two_wagers", vec![L::Fold, L::Call, L::AllIn(500)]), ("cap3_three_wagers", vec![L::Fold, L::Call])] {
        let c = case(name);
        let tree: proto::EffectiveTree = serde_json::from_value(c["tree"].clone()).unwrap();
        let history: Vec<proto::Action> = serde_json::from_value(c["history"].clone()).unwrap();
        let mut t = build(&tree, c["pot"].as_u64().unwrap() as u32, c["eff"].as_u64().unwrap() as u32, 0.0, 0, &history).unwrap();
        t.apply_history(&history_to_lib(&history)).unwrap();
        assert_eq!(t.available_actions(), &expected_after_prefix[..], "{name}");
        // The observed prefix survives the cap: at the node BEFORE each observed action, that action is still offered.
        // (Applying the whole line first would put the tree at the child AFTER the action, whose menu never contains it.)
        let mut line = history_to_lib(&history);
        while !line.is_empty() {
            let last = line.pop().unwrap();
            t.apply_history(&line).unwrap();
            assert!(t.available_actions().contains(&last), "{name}: {last:?} was removed by the cap at {line:?}");
        }
    }
}

#[test]
fn exact_size_insertion_no_prune() {
    let mut w = Worker::spawn(8); ready(&w);
    let flop = &fixture_lines("flop_cancel")[0];
    let ins = case("insert_73");
    w.send(&edit(flop, |v| { v["id"] = json!("110"); v["pot"] = json!(100); v["stack_oop"] = json!(500); v["stack_ip"] = json!(500);
        v["tree"] = ins["tree"].clone(); v["history"] = ins["history"].clone(); v["deadline_ms"] = json!(4000); v["extraction_margin_ms"] = json!(600); }));
    let r = result_of(&w, "110");
    assert!(r["status"] == "ok" || r["status"] == "best_so_far");
    let sol = &r["solution"];
    let root = sol["nodes"].as_array().unwrap().iter().find(|n| n["path"] == json!([])).unwrap();
    assert_eq!(root["actions"], json!([{"kind": "check"}, {"kind": "bet", "to": 50}, {"kind": "bet", "to": 73}]));
    let requested = &sol["nodes"][sol["requested"].as_u64().unwrap() as usize];
    assert_eq!((requested["path"].clone(), requested["actor"].as_str()), (json!([{"kind": "bet", "to": 73}]), Some("ip")));
    // the opponent's posterior after the 73 bet is not uniform: OOP's Bet(73) probability varies across its available combos
    let p73: Vec<f64> = (0..1326).filter(|i| root["available"][*i] == true).map(|i| root["probs"][i][2].as_f64().unwrap()).collect();
    let (min, max) = p73.iter().fold((1.0f64, 0.0f64), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    assert!(max - min > 0.05, "Bet(73) is uniform: {min}..{max}");
}

fn permute_card(c: Card, perm: [u8; 4]) -> Card { Card::new(c.rank(), perm[c.suit() as usize]) }
fn permute_range(r: &[f32], perm: [u8; 4]) -> Vec<f32> {
    let mut out = vec![0.0f32; 1326];
    for i in 0..1326 { let [a, b] = combo_cards(i as u16); out[combo_index(permute_card(a, perm), permute_card(b, perm)) as usize] = r[i]; }
    out
}

#[test]
fn suit_permutation_metamorphic() {
    let mut w = Worker::spawn(4); ready(&w);
    let base = &fixture_lines("river_two_combo")[0];
    let perm = [0u8, 2, 1, 3];   // swap diamonds and hearts
    for (k, board) in [vec!["Qs", "Jd", "7h", "3c", "2d"], vec!["Qs", "Qd", "7h", "3c", "2d"], vec!["Qs", "Js", "7s", "3s", "2s"]].into_iter().enumerate() {
        let id_a = format!("12{k}a"); let id_b = format!("12{k}b");
        w.send(&edit(base, |v| { v["id"] = json!(id_a); v["board"] = json!(board); }));
        let a = result_of(&w, &id_a);
        let req: Value = serde_json::from_str(base).unwrap();
        let pb: Vec<String> = board.iter().map(|s| permute_card(Card::parse(s).unwrap(), perm).to_string()).collect();
        let oop: Vec<f32> = req["oop_range"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        let ip: Vec<f32> = req["ip_range"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        w.send(&edit(base, |v| { v["id"] = json!(id_b); v["board"] = json!(pb); v["oop_range"] = json!(permute_range(&oop, perm)); v["ip_range"] = json!(permute_range(&ip, perm)); }));
        let b = result_of(&w, &id_b);
        assert!(a["status"] == "ok" && b["status"] == "ok");
        for (na, nb) in a["solution"]["nodes"].as_array().unwrap().iter().zip(b["solution"]["nodes"].as_array().unwrap()) {
            for i in 0..1326 {
                let [x, y] = combo_cards(i as u16);
                let j = combo_index(permute_card(x, perm), permute_card(y, perm)) as usize;
                assert_eq!(na["available"][i], nb["available"][j]);
                for (pa, pb) in na["probs"][i].as_array().unwrap().iter().zip(nb["probs"][j].as_array().unwrap()) { assert!((pa.as_f64().unwrap() - pb.as_f64().unwrap()).abs() <= 1e-4); }
            }
        }
    }
    let _ = Range1326([0.0; 1326]);
}

#[test]
fn pinned_example_fixture() {
    let mut w = Worker::spawn(16); ready(&w);
    let req = &fixture_lines("basic_turn_std_request")[0];
    let expected: Value = serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/solver/basic_0p3.json")).unwrap()).unwrap();
    w.send(req);
    let r = result_of(&w, "basic");
    assert_eq!(r["status"], "ok");
    let nodes = r["solution"]["nodes"].as_array().unwrap();
    let exp = expected["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), exp.len());
    for (n, e) in nodes.iter().zip(exp) {
        assert_eq!(n["path"], e["path"]);
        for i in 0..1326 {
            for a in 0..n["actions"].as_array().unwrap().len() {
                assert!((n["probs"][i][a].as_f64().unwrap() - e["probs"][i][a].as_f64().unwrap()).abs() <= 1e-3, "probs at {:?} combo {i} action {a}", n["path"]);
                assert!((n["ev_chips"][i][a].as_f64().unwrap() - e["ev_chips"][i][a].as_f64().unwrap()).abs() <= 1e-3 * 200.0, "ev at {:?} combo {i} action {a}", n["path"]);
            }
        }
    }
}
