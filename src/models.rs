use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, super::error::Error>;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum DragItem {
    Files(Vec<PathBuf>),
    Data {
        data: SharedData,
        #[serde(rename = "types")]
        mime_types: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum SharedData {
    Fixed(String),
    Map(HashMap<String, String>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Image {
    Base64(String),
    Raw(String),
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DragMode {
    #[default]
    Copy,
    Move,
}

#[derive(Debug, Deserialize, Default)]
pub struct DragOptions {
    #[serde(default)]
    pub mode: DragMode,
}

#[derive(Debug, Clone, Serialize)]
pub struct CursorPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize)]
pub enum DragResult {
    Dropped,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallbackResult {
    pub result: DragResult,
    #[serde(rename = "cursorPos")]
    pub cursor_pos: CursorPosition,
}
