use core_eval::*;
use proto::Card;
use std::path::PathBuf;
use std::sync::OnceLock;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/eval").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (run tools/gen_eval_oracle.py)", path.display()))
}

/// phevaluator rank (1..7462) -> our rank, built **once per test binary** from the full 5-card
/// oracle; panics on any inconsistency. Without the cache every test rebuilds a 2,598,960-hand map.
fn oracle_map() -> &'static [u16] {
    static MAP: OnceLock<Vec<u16>> = OnceLock::new();
    MAP.get_or_init(build_oracle_map)
}

fn build_oracle_map() -> Vec<u16> {
    let data = fixture("phevaluator_5card.bin");
    assert_eq!(data.len(), 2_598_960 * 2);
    let mut map: Vec<Option<u16>> = vec![None; 7463];
    let mut k = 0usize;
    for a in 0..52u8 { for b in (a + 1)..52 { for c in (b + 1)..52 { for d in (c + 1)..52 { for e in (d + 1)..52 {
        let oracle = u16::from_le_bytes([data[2 * k], data[2 * k + 1]]) as usize;
        k += 1;
        let ours = rank5(&[Card(a), Card(b), Card(c), Card(d), Card(e)]);
        match map[oracle] {
            None => map[oracle] = Some(ours),
            Some(prev) => assert_eq!(prev, ours, "phevaluator rank {oracle} maps to two different ranks"),
        }
    } } } } }
    assert_eq!(k, 2_598_960);
    (0..=7462).map(|o| if o == 0 { 0 } else { map[o].expect("every phevaluator rank occurs") }).collect()
}

#[test]
fn eval_vs_phevaluator_full_5card() {
    let map = oracle_map();
    for o in 1..7462 {
        assert!(map[o] > map[o + 1], "rank-order equivalence: phevaluator {o} (stronger) must map above {} ", o + 1);
    }
    assert_eq!(rank5(&["As", "Ks", "Qs", "Js", "Ts"].map(|s| s.parse().unwrap())), map[1]);
    assert_eq!(rank7(&["As", "Ks", "Qs", "Js", "Ts", "2c", "3d"].map(|s| s.parse().unwrap())), map[1]);
}

fn check_samples(name: &str, expected_records: usize) {
    let map = oracle_map();
    let data = fixture(name);
    assert_eq!(data.len(), expected_records * 9, "{name}: 9-byte records");
    for rec in data.chunks_exact(9) {
        let cards: [Card; 7] = std::array::from_fn(|i| Card(rec[i]));
        let oracle = u16::from_le_bytes([rec[7], rec[8]]) as usize;
        assert_eq!(rank7(&cards), map[oracle], "{cards:?}");
    }
}

#[test]
fn eval_vs_phevaluator_random_7card() { check_samples("phevaluator_7card_200k.bin", 200_000); }

/// Generate the input with `tools/.venv/Scripts/python tools/gen_eval_oracle.py --skip-5card --samples 10000000 --samples-name phevaluator_7card_10m.bin` (gitignored).
#[cfg(feature = "exhaustive")]
#[test]
fn eval_vs_phevaluator_random_7card_exhaustive() { check_samples("phevaluator_7card_10m.bin", 10_000_000); }
