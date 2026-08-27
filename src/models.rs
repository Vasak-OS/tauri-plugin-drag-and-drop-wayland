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

#[derive(Debug, Clone, Copy, Deserialize, Default)]
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
    /// La acción que negoció el destino: `"copy"` o `"move"`. Informativa.
    pub action: Option<String>,
    /// Si el destino pidió que el origen borre lo que entregó.
    ///
    /// **Esta es la señal que autoriza borrar, y no `action`.** Un destino puede
    /// elegir mover y después no pedir el borrado —porque falló al guardar, o
    /// porque cambió de idea—, y borrar por haber visto «move» sería perder un
    /// archivo que nadie copió a ninguna parte. GTK emite `drag-data-delete` sólo
    /// cuando la entrega salió bien y corresponde borrar.
    #[serde(rename = "sourceShouldDelete")]
    pub source_should_delete: bool,
    #[serde(rename = "cursorPos")]
    pub cursor_pos: CursorPosition,
}

#[cfg(test)]
mod diagnostico {
    use super::*;

    #[test]
    fn una_ruta_de_icono_se_lee_como_base64() {
        // `#[serde(untagged)]` prueba las variantes **en orden** y se queda con la
        // primera que encaje. `Base64(String)` y `Raw(String)` encajan las dos con
        // cualquier cadena, así que `Raw` es inalcanzable: una ruta de archivo
        // —que es lo que manda el gestor de archivos— se intenta decodificar como
        // base64, falla, y el arrastre nunca tiene icono.
        let como_json = serde_json::from_str::<Image>(r#""/usr/share/icons/x/32x32.png""#)
            .expect("deserializa");
        match como_json {
            Image::Base64(v) => println!("SE LEYÓ COMO BASE64: {v}"),
            Image::Raw(v) => panic!("se leyó como ruta: {v}"),
        }
    }

    #[test]
    fn los_datos_sueltos_se_aceptan_y_se_descartan() {
        // La API de JS ofrece `{ data, types }`, y el lado Rust lo convierte en
        // `None` y no hace nada: el arrastre sale sin ningún contenido.
        let d: DragItem = serde_json::from_str(r#"{"data":"hola","types":["text/plain"]}"#)
            .expect("deserializa");
        assert!(matches!(d, DragItem::Data { .. }));
    }
}
