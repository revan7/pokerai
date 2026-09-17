pub mod error;
pub mod service;
#[cfg(test)]
mod tests;

pub use error::AppError;
pub use service::{EnginePort, Op, Service};

pub fn run() {
    tauri::Builder::default().run(tauri::generate_context!()).expect("Tauri startup");
}
