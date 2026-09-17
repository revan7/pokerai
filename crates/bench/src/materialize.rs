use engine::tree::{materialize_at, Templates};
use proto::Action;

/// `--prefix oop:bet:73,ip:raise:200,oop:call` -> actor-labelled prefix.
///
/// R2: each comma-separated item must match a *complete* field slice — exactly `actor:action` for
/// a zero-arity action (check/call/fold) or exactly `actor:action:amount` for a wager — and no
/// field may be empty. Reading only the first two or three fields and discarding the rest (the
/// original bug) let a malformed item such as `oop:check:ip:bet:73` or `oop:bet:73:garbage`
/// silently become a different, valid-looking prefix step instead of a syntax error.
pub fn parse_prefix(s: &str) -> Result<Vec<(usize, Action)>, String> {
    if s.trim().is_empty() { return Ok(vec![]); }
    s.split(',').map(|raw_item| {
        let item = raw_item.trim();
        let parts: Vec<&str> = item.split(':').collect();
        let actor = match parts.first().copied() { Some("oop") => 0, Some("ip") => 1, _ => return Err(format!("bad actor in {item:?}: expected \"oop\" or \"ip\"")) };
        let action_name = match parts.get(1).copied() { Some(a) if !a.is_empty() => a, _ => return Err(format!("missing or empty action field in {item:?}")) };
        let zero_arity = |action: Action| if parts.len() == 2 { Ok(action) } else { Err(format!("{item:?}: {action_name} takes exactly \"actor:{action_name}\", got {} field(s)", parts.len())) };
        let wager = |make: fn(u32) -> Action| -> Result<Action, String> {
            if parts.len() != 3 { return Err(format!("{item:?}: {action_name} takes exactly \"actor:{action_name}:amount\", got {} field(s)", parts.len())); }
            let amount = parts[2];
            if amount.is_empty() { return Err(format!("empty amount field in {item:?}")); }
            let to = amount.parse::<u32>().map_err(|e| format!("bad amount {amount:?} in {item:?}: {e}"))?;
            Ok(make(to))
        };
        let action = match action_name {
            "check" => zero_arity(Action::Check)?, "call" => zero_arity(Action::Call)?, "fold" => zero_arity(Action::Fold)?,
            "bet" => wager(|to| Action::Bet { to })?, "raise" => wager(|to| Action::Raise { to })?, "allin" => wager(|to| Action::AllIn { to })?,
            other => return Err(format!("bad action {other:?} in {item:?}")),
        };
        Ok((actor, action))
    }).collect()
}

pub fn run(template: &str, pot: u32, eff: u32, prefix: &str) -> Result<String, String> {
    let t = Templates::get(template).ok_or(format!("unknown template {template}"))?;
    let b = materialize_at(t, pot, eff, &parse_prefix(prefix)?).map_err(|e| format!("{e:?}"))?;
    serde_json::to_string(&serde_json::json!({ "tree": b.tree, "history": b.history, "decision_path": b.decision_path })).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> Vec<(usize, Action)> { parse_prefix(s).unwrap() }
    fn err(s: &str) -> String { parse_prefix(s).unwrap_err() }

    /// R2: `oop:check:ip:bet:73` has no comma, so it is one field-slice `["oop","check","ip","bet","73"]`
    /// (5 fields) rather than two prefix steps. The old parser silently read only the first two fields
    /// (actor, action) and discarded the rest, turning a malformed single item into a valid one-step
    /// "oop check" prefix.
    #[test]
    fn surplus_fields_after_a_zero_arity_action_are_rejected() {
        let e = err("oop:check:ip:bet:73");
        assert!(e.contains("oop:check:ip:bet:73"), "{e}");
    }

    /// R2: `oop:bet:73:garbage` has 4 fields where a bet takes exactly 3 (actor:bet:amount); the old
    /// parser read only fields 0-2 and silently ignored the trailing `garbage`.
    #[test]
    fn surplus_field_after_a_wager_amount_is_rejected() {
        let e = err("oop:bet:73:garbage");
        assert!(e.contains("oop:bet:73:garbage"), "{e}");
    }

    /// R2: a wager with no amount field at all must still fail (regression control: this was already
    /// rejected before the fix, and must stay rejected after it).
    #[test]
    fn missing_amount_is_rejected() {
        let e = err("oop:bet");
        assert!(e.contains("oop:bet"), "{e}");
    }

    /// R2: an empty field (actor, action or amount) must be rejected rather than parsed as if absent.
    #[test]
    fn empty_fields_are_rejected() {
        for item in [":check", "oop::73", "oop:bet:", "oop:bet::"] {
            let e = err(item);
            assert!(e.contains(item), "{item}: {e}");
        }
    }

    /// R2 control: the explicitly supported empty prefix still parses to no steps.
    #[test]
    fn empty_prefix_is_still_supported() {
        assert_eq!(ok(""), vec![]);
        assert_eq!(ok("   "), vec![]);
    }

    /// R2 control: every supported action, for both actors, still parses to the exact step.
    #[test]
    fn every_supported_action_parses_for_both_actors() {
        assert_eq!(ok("oop:check"), vec![(0, Action::Check)]);
        assert_eq!(ok("oop:call"), vec![(0, Action::Call)]);
        assert_eq!(ok("oop:fold"), vec![(0, Action::Fold)]);
        assert_eq!(ok("oop:bet:50"), vec![(0, Action::Bet { to: 50 })]);
        assert_eq!(ok("oop:raise:100"), vec![(0, Action::Raise { to: 100 })]);
        assert_eq!(ok("oop:allin:200"), vec![(0, Action::AllIn { to: 200 })]);
        assert_eq!(ok("ip:check"), vec![(1, Action::Check)]);
        assert_eq!(ok("ip:call"), vec![(1, Action::Call)]);
        assert_eq!(ok("ip:fold"), vec![(1, Action::Fold)]);
        assert_eq!(ok("ip:bet:50"), vec![(1, Action::Bet { to: 50 })]);
        assert_eq!(ok("ip:raise:100"), vec![(1, Action::Raise { to: 100 })]);
        assert_eq!(ok("ip:allin:200"), vec![(1, Action::AllIn { to: 200 })]);
        assert_eq!(ok("oop:bet:73,ip:raise:200,oop:call"), vec![(0, Action::Bet { to: 73 }), (1, Action::Raise { to: 200 }), (0, Action::Call)]);
    }

    /// R2 control: an unparseable amount is still rejected, and the message names the item.
    #[test]
    fn non_numeric_amount_is_rejected() {
        let e = err("oop:bet:abc");
        assert!(e.contains("oop:bet:abc"), "{e}");
    }
}
