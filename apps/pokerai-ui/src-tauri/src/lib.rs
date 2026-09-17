pub mod commands;
pub mod error;
pub mod service;
#[cfg(test)]
mod tests;

pub use error::AppError;
pub use service::{EnginePort, Op, Service};

/// Wires the fourteen commands into a Tauri builder. Only exercised through
/// Tauri's generated `invoke_handler` glue and by the `tests` module today
/// (`run()` gains a real `EnginePort` in a later task); kept `pub` so the
/// dead-code lint treats it as reachable crate API rather than unused code.
pub fn configure<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    service: service::Service,
) -> tauri::Builder<R> {
    builder.manage(service).invoke_handler(tauri::generate_handler![
        commands::set_game_config,
        commands::begin_hand,
        commands::set_hero_cards,
        commands::apply_action,
        commands::set_board,
        commands::undo,
        commands::recommend,
        commands::cancel,
        commands::finish_hand,
        commands::abandon_hand,
        commands::set_seat_tag,
        commands::presolver_status,
        commands::presolver_pause,
        commands::presolver_resume
    ])
}

pub fn run() {
    tauri::Builder::default().run(tauri::generate_context!()).expect("Tauri startup");
}
