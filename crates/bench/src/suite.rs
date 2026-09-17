use proto::{Action, Card, Rake, Street};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Spot {
    pub id: String, pub template_id: String, pub root_street: Street, pub board: Vec<Card>,
    pub oop_range: String, pub ip_range: String, pub pot: u32, pub stack_oop: u32, pub stack_ip: u32,
    pub rake: Rake, pub history: Vec<Action>, pub target_bp: u16,
    /// "r8_uniform" (this plan's labelled interim) | "chart_replay" (the §13.5 baseline, plan 3 bundles + plan 4 Task 20) | "store_replay" (V9)
    pub range_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Suite { pub suite: String, pub spots: Vec<Spot> }

impl Suite {
    pub fn load(path: &Path) -> Result<Suite, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn suite_roundtrip() {
        let s = Suite { suite: "river_std".into(), spots: vec![Spot { id: "srp100_dry".into(), template_id: "river_std_v1".into(), root_street: proto::Street::River,
            board: "Kh7d2c4d9s".as_bytes().chunks(2).map(|c| proto::Card::parse(std::str::from_utf8(c).unwrap()).unwrap()).collect(),
            oop_range: "AA".into(), ip_range: "KK".into(), pot: 241, stack_oop: 882, stack_ip: 882, rake: proto::Rake::TimeCharge, history: vec![], target_bp: 50, range_source: "r8_uniform".into() }] };
        let dir = std::env::temp_dir().join(format!("bench_suite_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("river_std.json");
        s.save(&p).unwrap();
        let back = Suite::load(&p).unwrap();
        assert_eq!(back.spots[0].board.len(), 5);
        assert_eq!(back.spots[0].id, "srp100_dry");
    }
}
