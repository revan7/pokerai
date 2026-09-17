use engine::tree::{materialize_at, Templates};
use proto::Action;

/// `--prefix oop:bet:73,ip:raise:200,oop:call` -> actor-labelled prefix.
pub fn parse_prefix(s: &str) -> Result<Vec<(usize, Action)>, String> {
    if s.trim().is_empty() { return Ok(vec![]); }
    s.split(',').map(|item| {
        let parts: Vec<&str> = item.trim().split(':').collect();
        let actor = match parts.first().copied() { Some("oop") => 0, Some("ip") => 1, _ => return Err(format!("bad actor in {item}")) };
        let to = |i: usize| parts.get(i).ok_or(format!("missing amount in {item}"))?.parse::<u32>().map_err(|e| e.to_string());
        let action = match parts.get(1).copied() {
            Some("check") => Action::Check, Some("call") => Action::Call, Some("fold") => Action::Fold,
            Some("bet") => Action::Bet { to: to(2)? }, Some("raise") => Action::Raise { to: to(2)? }, Some("allin") => Action::AllIn { to: to(2)? },
            _ => return Err(format!("bad action in {item}")),
        };
        Ok((actor, action))
    }).collect()
}

pub fn run(template: &str, pot: u32, eff: u32, prefix: &str) -> Result<String, String> {
    let t = Templates::get(template).ok_or(format!("unknown template {template}"))?;
    let b = materialize_at(t, pot, eff, &parse_prefix(prefix)?).map_err(|e| format!("{e:?}"))?;
    serde_json::to_string(&serde_json::json!({ "tree": b.tree, "history": b.history, "decision_path": b.decision_path })).map_err(|e| e.to_string())
}
