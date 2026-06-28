use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

mod commands;
mod error;
mod models;

pub use error::Error;
pub use models::*;

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("drag-and-drop-wayland")
        .invoke_handler(tauri::generate_handler![commands::start_drag])
        .build()
}
