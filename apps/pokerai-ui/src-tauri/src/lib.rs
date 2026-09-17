mod error;
mod service;
#[cfg(test)]
mod tests;

pub fn run() {
    tauri::Builder::default().run(tauri::generate_context!()).expect("Tauri startup");
}
