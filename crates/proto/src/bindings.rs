use std::{collections::BTreeMap, fs, io, path::Path};
use ts_rs::{Config, TS, TypeVisitor};
struct Registry { cfg: Config, declarations: BTreeMap<String, String> }
impl TypeVisitor for Registry {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        if T::output_path().is_none() { return; }
        let name = T::ident(&self.cfg);
        if self.declarations.contains_key(&name) { return; }
        self.declarations.insert(name, format!("export {}", T::decl(&self.cfg)));
        T::visit_dependencies(self);
    }
}
pub fn declarations() -> String {
    let mut r = Registry { cfg: Config::new().with_large_int("number"),
        declarations: BTreeMap::new() };
    macro_rules! roots { ($($t:ty),+ $(,)?) => { $(r.visit::<$t>();)+ }; }
    roots!(crate::Card, crate::Range1326, crate::Position, crate::Seat,
        crate::Street, crate::GameConfig, crate::HandConfig, crate::BeginHand,
        crate::HandState, crate::Derived, crate::Action, crate::TakenAction,
        crate::HandPhase, crate::LegalAction, crate::StreetRootSnapshot,
        crate::SolveInput, crate::DecisionIdentity, crate::Recommendation,
        crate::RecommendationEvent, crate::Coverage, crate::ApproxReason,
        crate::UnsupportedReason, crate::Unavailable, crate::Availability,
        crate::EquityEstimate, crate::EquitySummary, crate::PotShares,
        crate::Assumptions, crate::ExperimentalHu, crate::EffectiveTree,
        crate::MaterializedNode, crate::worker::EngineMessage,
        crate::worker::WorkerMessage, crate::worker::StreetSolution,
        crate::worker::NodeStrategy, crate::worker::NodeLock,
        crate::worker::WorkerError, crate::CardParseError, crate::UtgStraddle,
        crate::Rake, crate::SeatConfig, crate::SeatTag, crate::QuickFact,
        crate::SolverPrefs, crate::CompleteReason, crate::Pot, crate::ActionAdvice,
        crate::EquityMethod, crate::ExploitAdvice, crate::Phase,
        crate::PlayerMenus, crate::SideMenu, crate::MenuSize,
        crate::worker::SolveRequest, crate::worker::Ready,
        crate::worker::AckStatus, crate::worker::Stage, crate::worker::ResultStatus);
    let mut text = String::from("// Generated from proto by ts-rs; do not edit.\n");
    for decl in r.declarations.values() { text.push_str(decl); text.push('\n'); }
    // Rust aliases have no distinct TypeId; obtain the RHS from the Rust alias itself.
    for (name,rhs) in [
        ("ComboIndex",<crate::ComboIndex as TS>::inline(&r.cfg)),
        ("ChipPath",<crate::ChipPath as TS>::inline(&r.cfg)),
        ("OrdinalPath",<crate::OrdinalPath as TS>::inline(&r.cfg)),
    ] {text.push_str(&format!("export type {name} = {rhs};\n"));}

    text
}
pub fn write_to(path: &Path) -> io::Result<()> {
    let text = declarations();
    if fs::read_to_string(path).ok().as_deref() != Some(&text) {
        fs::create_dir_all(path.parent().expect("types output has a parent"))?;
        fs::write(path, text)?;
    }
    Ok(())
}
