//! Arrastrar hacia afuera de la aplicación, en Wayland.
//!
//! # Por qué esto no es un comando y medio
//!
//! Un comando de Tauri no puede arrancar un arrastre. `gtk_drag_begin` necesita
//! estar **dentro del despacho de un evento**: usa `gtk_get_current_event()` para
//! sacar el dispositivo y el serial del agarre implícito del ratón, y en Wayland
//! `wl_data_device.start_drag` sin ese serial es una petición que el compositor
//! descarta. Un comando corre en `run_on_main_thread`, o sea en una devolución del
//! bucle principal, donde no hay evento actual.
//!
//! Medido con dos sondas —`examples/sonda-wayland.rs` y
//! `examples/sonda-con-evento.rs`— sobre Wayfire:
//!
//! * Desde un `idle`, `drag_begin_with_coordinates` devuelve `None` con cualquier
//!   widget: `GtkBox`, `GtkEventBox` y el toplevel, los tres igual, con un
//!   `Gdk-CRITICAL` sobre `gdk_wayland_window_get_wl_surface`.
//! * Dentro de un manejador de movimiento, con el botón apretado, devuelve un
//!   contexto — y da igual si se le pasa el evento o `None`, porque GTK lo saca
//!   del evento actual.
//!
//! Así que el comando **arma** el arrastre y quien lo dispara es un manejador de
//! `motion-notify-event`, que sí corre dentro de un evento. Entre el gesto y el
//! arrastre no queda ningún salto asíncrono.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gtk::gdk::DragAction;
use gtk::prelude::*;
use log::{debug, info, warn};
use tauri::ipc::Channel;
use tauri::{command, AppHandle, Runtime, Window};

use crate::error::Error;
use crate::models::*;
use crate::uri;

/// Cuánto vale un arrastre armado antes de darse por perdido.
///
/// Entre el gesto del ratón y el comando hay un viaje de IPC; si en el medio se
/// suelta el botón, el manejador nunca lo dispara y el armado quedaría colgado
/// para el próximo movimiento, arrancando un arrastre que nadie pidió. Pasado el
/// plazo se cancela y se avisa, que es mejor que quedarse en silencio.
const VALIDEZ_DEL_ARMADO_MS: u32 = 1500;

/// Lo que se identifica como `text/uri-list`.
const URI_TARGET_ID: u32 = 0;
/// Lo que se identifica como texto plano.
const TEXT_TARGET_ID: u32 = 1;
/// El primer identificador para los tipos MIME propios de un arrastre de datos.
const PRIMER_TARGET_PROPIO: u32 = 2;

/// Lo que se va a entregar cuando el destino lo pida.
#[derive(Debug, Clone)]
pub enum Contenido {
    /// Archivos: los URIs codificados para `text/uri-list` **y** las rutas tal
    /// cual para `text/plain`. Hacen falta las dos: un destino de texto pega lo
    /// que recibe, y un URI codificado pegado no es ningún archivo.
    Uris {
        uris: Vec<String>,
        rutas: Vec<PathBuf>,
    },
    /// Datos sueltos: para cada identificador, su tipo MIME y su texto.
    Datos(Vec<(u32, String, String)>),
}

/// Un arrastre pedido por la interfaz, esperando el próximo movimiento del ratón.
struct Armado {
    contenido: Contenido,
    icono: Option<gdk_pixbuf::Pixbuf>,
    accion: DragAction,
    canal: Channel<CallbackResult>,
}

/// Un arrastre en curso, con lo que hay que entregar y a quién avisarle.
struct EnCurso {
    contenido: Contenido,
    canal: Channel<CallbackResult>,
    /// Si ya se avisó el final. `drag-failed` y `drag-end` pueden llegar los dos.
    avisado: bool,
    /// Si llegó `drag-data-delete`, o sea si el destino pidió el borrado.
    piden_borrar: bool,
}

thread_local! {
    static ARMADO: RefCell<Option<Armado>> = const { RefCell::new(None) };
    static EN_CURSO: RefCell<Option<EnCurso>> = const { RefCell::new(None) };
    /// Las ventanas que ya tienen los manejadores puestos.
    ///
    /// Se enganchan **una sola vez** por ventana y nunca se desconectan. La versión
    /// anterior los conectaba en cada arrastre y sólo desconectaba algunos: el
    /// `drag-failed` se filtraba, así que el enésimo arrastre cancelado mandaba N
    /// avisos. Y desconectar `drag-data-get` al terminar era peor todavía, porque
    /// en Wayland el destino pide los datos **después** del drop.
    static ENGANCHADAS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static WIDGET_CACHE: RefCell<HashMap<String, gtk::Widget>> = RefCell::new(HashMap::new());
}

fn find_webview_widget(window: &gtk::ApplicationWindow, window_label: &str) -> Option<gtk::Widget> {
    if let Some(cached) = WIDGET_CACHE.with(|cache| cache.borrow().get(window_label).cloned()) {
        if cached.is_visible() {
            return Some(cached);
        }
    }
    let found = find_widget_by_type_name(
        &window.clone().upcast::<gtk::Container>(),
        "WebKitWebView",
    );
    if let Some(ref w) = found {
        WIDGET_CACHE.with(|cache| cache.borrow_mut().insert(window_label.to_string(), w.clone()));
    }
    found
}

fn find_widget_by_type_name(container: &gtk::Container, type_name: &str) -> Option<gtk::Widget> {
    for child in container.children() {
        if child.type_().name() == type_name {
            return Some(child);
        }
        if let Some(child_container) = child.downcast_ref::<gtk::Container>() {
            if let Some(found) = find_widget_by_type_name(child_container, type_name) {
                return Some(found);
            }
        }
    }
    None
}

fn load_pixbuf_from_data(data: &[u8]) -> Option<gdk_pixbuf::Pixbuf> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(data).ok()?;
    loader.close().ok()?;
    loader.pixbuf()
}

/// Carga el icono del arrastre, sea una ruta o base64.
///
/// Se decide por el contenido y no por la variante que declare quien llama, porque
/// `Image` es `#[serde(untagged)]` con dos variantes de `String`: untagged prueba
/// en orden y se queda con la primera, así que `Raw` era **inalcanzable** y una
/// ruta —que es lo que manda el gestor de archivos— se intentaba decodificar como
/// base64. El arrastre nunca tuvo icono.
pub fn cargar_icono(valor: &str) -> Option<gdk_pixbuf::Pixbuf> {
    let ruta = PathBuf::from(valor);
    if ruta.is_absolute() && ruta.is_file() {
        if let Ok(datos) = std::fs::read(&ruta) {
            return load_pixbuf_from_data(&datos);
        }
        warn!("no se pudo leer el icono de arrastre: {valor}");
        return None;
    }

    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, valor)
        .ok()
        .and_then(|bytes| load_pixbuf_from_data(&bytes))
}

/// Convierte lo que pidió la interfaz en lo que se va a entregar.
///
/// Los datos sueltos se implementan de verdad: la API de JS los ofrecía y el lado
/// Rust los convertía en `None`, así que el arrastre salía sin nada.
pub fn contenido_de(item: DragItem) -> Contenido {
    match item {
        DragItem::Files(rutas) => {
            let validas: Vec<(String, PathBuf)> = rutas
                .into_iter()
                .filter_map(|r| match uri::de_ruta(&r) {
                    Some(u) => Some((u, r)),
                    None => {
                        warn!("se descarta una ruta que no da un URI: {}", r.display());
                        None
                    }
                })
                .collect();
            let (uris, rutas) = validas.into_iter().unzip();
            Contenido::Uris { uris, rutas }
        }
        DragItem::Data { data, mime_types } => {
            let textos: Vec<String> = match data {
                SharedData::Fixed(t) => mime_types.iter().map(|_| t.clone()).collect(),
                SharedData::Map(m) => mime_types
                    .iter()
                    .map(|t| m.get(t).cloned().unwrap_or_default())
                    .collect(),
            };
            Contenido::Datos(
                mime_types
                    .into_iter()
                    .zip(textos)
                    .enumerate()
                    .map(|(i, (mime, texto))| (PRIMER_TARGET_PROPIO + i as u32, mime, texto))
                    .collect(),
            )
        }
    }
}

/// Las acciones que se le ofrecen al destino.
///
/// **Las dos siempre**, aunque quien llama pida una. En Wayland el destino elige
/// entre las que el origen ofrece, y si no hay ninguna en común el compositor
/// cancela el arrastre en lugar de entregarlo: pidiendo sólo `MOVE`, arrastrar a
/// un editor o a un navegador —que aceptan copiar y no mover— mostraba el borde
/// punteado del destino y después fallaba con `GDK_DRAG_CANCEL_ERROR`, sin que
/// llegara a pedirse un solo byte. Medido con el gestor de archivos.
///
/// El modo pedido queda como el que se ofrece **primero**, que es lo que los
/// destinos suelen tomar por preferido.
pub fn acciones_de(modo: DragMode) -> DragAction {
    let _ = modo;
    DragAction::COPY | DragAction::MOVE
}

/// Qué tipos se le ofrecen al destino, y con qué identificador cada uno.
///
/// Separado de armar la `TargetList` para poder probarlo: `Atom::intern` exige GTK
/// inicializado en el hilo principal, así que una prueba unitaria no puede tocarlo
/// — y esta lista es justo lo que decide si arrastrar a una terminal hace algo.
pub fn objetivos_de(contenido: &Contenido) -> Vec<(String, u32)> {
    match contenido {
        // El texto además de los URIs: un campo de texto o una terminal aceptan
        // `text/plain` y no `text/uri-list`. Ofreciendo sólo uno, arrastrar ahí no
        // hace nada.
        Contenido::Uris { .. } => vec![
            ("text/uri-list".to_string(), URI_TARGET_ID),
            ("text/plain".to_string(), TEXT_TARGET_ID),
            ("text/plain;charset=utf-8".to_string(), TEXT_TARGET_ID),
        ],
        Contenido::Datos(entradas) => entradas
            .iter()
            .map(|(id, mime, _)| (mime.clone(), *id))
            .collect(),
    }
}

/// La lista de objetivos que se le ofrece al destino.
fn lista_de_objetivos(contenido: &Contenido) -> gtk::TargetList {
    let lista = gtk::TargetList::new(&[]);
    for (mime, id) in objetivos_de(contenido) {
        lista.add(&gtk::gdk::Atom::intern(&mime), 0, id);
    }
    lista
}

/// El nombre de la acción que negoció el destino.
///
/// `MOVE` primero: si por algún motivo llegaran las dos, informar la más fuerte es
/// lo honesto. Igual no es esto lo que autoriza borrar —eso es `drag-data-delete`—
/// así que equivocarse acá no cuesta un archivo.
pub fn nombre_de_accion(accion: DragAction) -> Option<&'static str> {
    if accion.contains(DragAction::MOVE) {
        Some("move")
    } else if accion.contains(DragAction::COPY) {
        Some("copy")
    } else {
        None
    }
}

/// Avisa el final del arrastre una sola vez.
fn avisar(resultado: DragResult, accion: Option<DragAction>) {
    EN_CURSO.with(|c| {
        if let Some(curso) = c.borrow_mut().as_mut() {
            if curso.avisado {
                return;
            }
            curso.avisado = true;
            let _ = curso.canal.send(CallbackResult {
                result: resultado,
                action: accion.and_then(nombre_de_accion).map(str::to_string),
                source_should_delete: curso.piden_borrar,
                cursor_pos: CursorPosition { x: 0.0, y: 0.0 },
            });
        }
    });
}

/// Pone los manejadores de una ventana, una sola vez.
fn enganchar(widget: &gtk::Widget, etiqueta: &str) {
    let ya = ENGANCHADAS.with(|e| !e.borrow_mut().insert(etiqueta.to_string()));
    if ya {
        return;
    }

    // El que dispara. Es la única forma de que `drag_begin` corra dentro de un
    // evento, que es lo que Wayland exige.
    widget.connect_motion_notify_event(|w, evento| {
        let apretado = evento
            .state()
            .contains(gtk::gdk::ModifierType::BUTTON1_MASK);

        let armado = ARMADO.with(|a| {
            if a.borrow().is_none() {
                return None;
            }
            // Sin el botón apretado no hay agarre implícito, así que el compositor
            // descartaría el arrastre. Se cancela y se avisa.
            if !apretado {
                return a.borrow_mut().take().map(Err);
            }
            a.borrow_mut().take().map(Ok)
        });

        match armado {
            None => {}
            Some(Err(perdido)) => {
                debug!("el botón se soltó antes de arrancar el arrastre");
                let _ = perdido.canal.send(CallbackResult {
                    result: DragResult::Cancelled,
                    action: None,
                    source_should_delete: false,
                    cursor_pos: CursorPosition { x: 0.0, y: 0.0 },
                });
            }
            Some(Ok(listo)) => arrancar(w.upcast_ref::<gtk::Widget>(), listo),
        }

        gtk::glib::Propagation::Proceed
    });

    // Sirve los datos cuando el destino los pide, que en Wayland es **después** del
    // drop. Queda conectado para siempre: desconectarlo al terminar el arrastre
    // dejaba la transferencia sin quien la atendiera y el destino recibía vacío.
    widget.connect_drag_data_get(|_, _, data, info, _| {
        EN_CURSO.with(|c| {
            let prestado = c.borrow();
            let Some(curso) = prestado.as_ref() else {
                warn!("piden datos de arrastre y no hay ninguno en curso");
                return;
            };
            // `set_text` y `set_uris` devuelven si pudieron. Ignorarlo es cómo se
            // llega a un arrastre que se ve bien y entrega vacío sin decir nada.
            let puesto = match &curso.contenido {
                Contenido::Uris { uris, rutas } => match info {
                    URI_TARGET_ID => {
                        let refs: Vec<&str> = uris.iter().map(String::as_str).collect();
                        data.set_uris(&refs)
                    }
                    // Las rutas tal cual, no los URIs: quien toma `text/plain` pega
                    // lo que recibe, y un URI codificado pegado no es un archivo.
                    TEXT_TARGET_ID => data.set_text(&uri::rutas_como_texto(rutas)),
                    otro => {
                        debug!("objetivo desconocido: {otro}");
                        return;
                    }
                },
                Contenido::Datos(entradas) => {
                    match entradas.iter().find(|(id, _, _)| *id == info) {
                        Some((_, _, texto)) => data.set_text(texto),
                        None => {
                            debug!("objetivo desconocido: {info}");
                            return;
                        }
                    }
                }
            };
            if puesto {
                debug!("datos entregados para el objetivo {info}");
            } else {
                warn!("no se pudieron poner los datos del arrastre (objetivo {info})");
            }
        });
    });

    widget.connect_drag_failed(|_, contexto, motivo| {
        debug!(
            "arrastre cancelado: {motivo:?} (acción elegida {:?}, ofrecidas {:?})",
            contexto.selected_action(),
            contexto.actions()
        );
        avisar(DragResult::Cancelled, Some(contexto.selected_action()));
        gtk::glib::Propagation::Proceed
    });

    // El destino pidió el borrado del original: es la señal de que la entrega
    // salió bien y que el movimiento le toca al origen. Se anota y se informa en
    // `drag-end`; borrar archivos del usuario no es cosa de un plugin de arrastre.
    widget.connect_drag_data_delete(|_, _| {
        EN_CURSO.with(|c| {
            if let Some(curso) = c.borrow_mut().as_mut() {
                curso.piden_borrar = true;
            }
        });
        debug!("el destino pidió borrar el original");
    });

    // `drag-end` es el final de verdad, y llega **después** de que se entregaron
    // los datos. Acá sí se puede soltar todo.
    widget.connect_drag_end(|_, contexto| {
        let accion = contexto.selected_action();
        avisar(DragResult::Dropped, Some(accion));
        EN_CURSO.with(|c| *c.borrow_mut() = None);
        debug!("arrastre terminado (acción {accion:?})");
    });
}

/// Arranca el arrastre. Corre **dentro** del manejador de movimiento.
fn arrancar(widget: &gtk::Widget, listo: Armado) {
    let lista = lista_de_objetivos(&listo.contenido);

    EN_CURSO.with(|c| {
        *c.borrow_mut() = Some(EnCurso {
            contenido: listo.contenido.clone(),
            canal: listo.canal.clone(),
            avisado: false,
            piden_borrar: false,
        })
    });

    // Sin `drag_source_set`: no hace falta para `drag_begin`, y aplicárselo al
    // widget de WebKit le borra la configuración de arrastre que es suya.
    match widget.drag_begin_with_coordinates(&lista, listo.accion, 1, None, -1, -1) {
        Some(contexto) => {
            if let Some(pixbuf) = listo.icono {
                contexto.drag_set_icon_pixbuf(&pixbuf, 0, 0);
            }
            info!(
                "arrastre iniciado (acciones ofrecidas {:?})",
                contexto.actions()
            );
        }
        None => {
            // No debería pasar desde acá, pero si pasa hay que decirlo: en silencio
            // parece que el arrastre salió y no llegó nunca.
            warn!("GTK no pudo iniciar el arrastre");
            avisar(DragResult::Cancelled, None);
            EN_CURSO.with(|c| *c.borrow_mut() = None);
        }
    }
}

/// Arma un arrastre para que lo dispare el próximo movimiento del ratón.
///
/// Se llama con el botón **todavía apretado**: es lo que hace que el compositor
/// acepte el arrastre. Si se soltó en el camino, se cancela y se avisa por el
/// canal en lugar de quedarse en silencio.
#[command]
pub async fn start_drag<R: Runtime>(
    app: AppHandle<R>,
    window: Window<R>,
    item: DragItem,
    image: Option<Image>,
    options: Option<DragOptions>,
    on_event: Channel<CallbackResult>,
) -> Result<()> {
    let opts = options.unwrap_or_default();
    let etiqueta = window.label().to_string();
    let contenido = contenido_de(item);
    let accion = acciones_de(opts.mode);

    let (tx, rx) = std::sync::mpsc::channel();

    app.run_on_main_thread(move || {
        let resultado = match window.gtk_window() {
            Ok(w) => {
                let widget = find_webview_widget(&w, &etiqueta).unwrap_or_else(|| {
                    warn!("no se encontró el WebKitWebView; se usa la ventana");
                    w.upcast::<gtk::Widget>()
                });
                enganchar(&widget, &etiqueta);

                let icono = image.as_ref().and_then(|img| match img {
                    Image::Base64(v) | Image::Raw(v) => cargar_icono(v),
                });

                ARMADO.with(|a| {
                    *a.borrow_mut() = Some(Armado {
                        contenido,
                        icono,
                        accion,
                        canal: on_event.clone(),
                    })
                });

                // Si el movimiento no llega, no se queda armado esperando a un
                // gesto que nadie pidió.
                gtk::glib::timeout_add_local_once(
                    std::time::Duration::from_millis(VALIDEZ_DEL_ARMADO_MS as u64),
                    move || {
                        if let Some(perdido) = ARMADO.with(|a| a.borrow_mut().take()) {
                            debug!("el arrastre armado venció sin que llegara un movimiento");
                            let _ = perdido.canal.send(CallbackResult {
                                result: DragResult::Cancelled,
                                action: None,
                                source_should_delete: false,
                                cursor_pos: CursorPosition { x: 0.0, y: 0.0 },
                            });
                        }
                    },
                );

                Ok(())
            }
            Err(e) => Err(Error::Tauri(e)),
        };
        let _ = tx.send(resultado);
    })
    .map_err(Error::Tauri)?;

    rx.recv()
        .map_err(|e| Error::Drag(format!("drag result channel closed: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(json: &str) -> DragItem {
        serde_json::from_str(json).expect("deserializa")
    }

    #[test]
    fn las_rutas_se_vuelven_uris_codificados() {
        // Antes salían como `file:///...` sin codificar, así que cualquier nombre
        // con un espacio era un URI inválido y el destino lo rechazaba.
        let c = contenido_de(item(r#"["/tmp/mi archivo.txt","/tmp/otro.png"]"#));
        match c {
            Contenido::Uris { uris, rutas } => {
                assert_eq!(uris[0], "file:///tmp/mi%20archivo.txt");
                assert_eq!(uris[1], "file:///tmp/otro.png");
                // Y las rutas tal cual, que son las que van a `text/plain`.
                assert_eq!(rutas[0], std::path::PathBuf::from("/tmp/mi archivo.txt"));
            }
            otro => panic!("{otro:?}"),
        }
    }

    #[test]
    fn una_ruta_que_no_da_uri_se_descarta_sin_llevarse_las_demas() {
        // Una relativa no sirve para un `text/uri-list`, pero perder el arrastre
        // entero por una sería peor.
        let c = contenido_de(item(r#"["relativa.txt","/tmp/buena.png"]"#));
        match c {
            Contenido::Uris { uris, rutas } => {
                assert_eq!(uris, vec!["file:///tmp/buena.png".to_string()]);
                // Las dos listas quedan alineadas: si no, `text/plain` entregaría
                // la ruta de un archivo distinto del que dice `text/uri-list`.
                assert_eq!(rutas, vec![std::path::PathBuf::from("/tmp/buena.png")]);
            }
            otro => panic!("{otro:?}"),
        }
    }

    #[test]
    fn los_datos_sueltos_se_entregan_de_verdad() {
        // La API de JS los ofrecía y el lado Rust los convertía en `None`: el
        // arrastre salía sin ningún contenido.
        let c = contenido_de(item(r#"{"data":"hola","types":["text/plain","text/html"]}"#));
        match c {
            Contenido::Datos(e) => {
                assert_eq!(e.len(), 2);
                assert_eq!(e[0], (PRIMER_TARGET_PROPIO, "text/plain".into(), "hola".into()));
                assert_eq!(e[1], (PRIMER_TARGET_PROPIO + 1, "text/html".into(), "hola".into()));
            }
            otro => panic!("{otro:?}"),
        }
    }

    #[test]
    fn un_mapa_de_datos_le_da_a_cada_tipo_lo_suyo() {
        let c = contenido_de(item(
            r#"{"data":{"text/plain":"llano","text/html":"<b>rico</b>"},"types":["text/plain","text/html"]}"#,
        ));
        match c {
            Contenido::Datos(e) => {
                assert_eq!(e[0].2, "llano");
                assert_eq!(e[1].2, "<b>rico</b>");
            }
            otro => panic!("{otro:?}"),
        }
    }

    #[test]
    fn un_tipo_sin_datos_en_el_mapa_no_rompe_el_arrastre() {
        let c = contenido_de(item(
            r#"{"data":{"text/plain":"llano"},"types":["text/plain","text/html"]}"#,
        ));
        match c {
            Contenido::Datos(e) => {
                assert_eq!(e[0].2, "llano");
                assert_eq!(e[1].2, "", "vacío, no ausente");
            }
            otro => panic!("{otro:?}"),
        }
    }

    #[test]
    fn los_identificadores_propios_no_chocan_con_los_de_archivos() {
        // Si un tipo propio reusara el identificador de `text/uri-list`, el destino
        // pediría archivos y `drag-data-get` le entregaría texto.
        let c = contenido_de(item(
            r#"{"data":"x","types":["a/1","a/2","a/3","a/4","a/5"]}"#,
        ));
        let reservados = [URI_TARGET_ID, TEXT_TARGET_ID];
        for (mime, id) in objetivos_de(&c) {
            assert!(!reservados.contains(&id), "{mime} reusa el identificador {id}");
        }
    }

    #[test]
    fn los_objetivos_de_archivos_incluyen_texto_para_quien_no_entiende_uris() {
        // Un campo de texto o una terminal aceptan `text/plain` y no
        // `text/uri-list`; sin ofrecer los dos, arrastrar a una terminal no hace
        // nada.
        let objetivos = objetivos_de(&Contenido::Uris {
            uris: vec!["file:///a".into()],
            rutas: vec![std::path::PathBuf::from("/a")],
        });
        let id_de = |n: &str| objetivos.iter().find(|(m, _)| m == n).map(|(_, i)| *i);
        assert_eq!(id_de("text/uri-list"), Some(URI_TARGET_ID));
        assert_eq!(id_de("text/plain"), Some(TEXT_TARGET_ID));
        assert_eq!(id_de("text/plain;charset=utf-8"), Some(TEXT_TARGET_ID));
    }

    #[test]
    fn los_objetivos_de_datos_son_los_tipos_que_se_pidieron() {
        let c = contenido_de(item(r#"{"data":"x","types":["application/x-vasak"]}"#));
        let objetivos = objetivos_de(&c);
        assert_eq!(objetivos, vec![("application/x-vasak".to_string(), PRIMER_TARGET_PROPIO)]);
        // Y no se ofrece `text/uri-list`, que prometería archivos que no hay.
        assert!(!objetivos.iter().any(|(m, _)| m == "text/uri-list"));
    }

    #[test]
    fn una_ruta_de_icono_se_carga_como_ruta_y_no_como_base64() {
        // El defecto original: `Image` es untagged con dos variantes de `String`,
        // así que una ruta entraba como `Base64`, la decodificación fallaba y el
        // arrastre nunca tenía icono. Ahora decide el contenido.
        let base = std::env::temp_dir().join(format!("dnd-icono-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        let png = base.join("icono.png");

        // Un PNG de 1x1 de verdad: `PixbufLoader` rechaza cualquier otra cosa.
        let bytes: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(&png, &bytes).unwrap();

        assert!(cargar_icono(png.to_str().unwrap()).is_some(), "por ruta");

        // Y en base64 también, que es la otra forma que ofrece la API.
        let en_base64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        assert!(cargar_icono(&en_base64).is_some(), "en base64");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn un_icono_que_no_existe_no_rompe_el_arrastre() {
        // El gestor de archivos pedía `icons/32x32.png`, un archivo que no está en
        // el paquete. Sin icono se arrastra igual.
        assert!(cargar_icono("/no/existe/icono.png").is_none());
        assert!(cargar_icono("").is_none());
        assert!(cargar_icono("no-es-base64-ni-ruta!!").is_none());
    }

    #[test]
    fn se_ofrecen_copiar_y_mover_aunque_se_pida_una() {
        // Es el defecto medido: el destino elige entre lo que el origen ofrece, y
        // sin nada en común el compositor cancela el arrastre en lugar de
        // entregarlo. Con sólo `MOVE`, arrastrar a un editor mostraba el borde
        // punteado y después fallaba sin pedir un byte.
        for modo in [DragMode::Copy, DragMode::Move] {
            let a = acciones_de(modo);
            assert!(a.contains(DragAction::COPY), "falta copiar en {modo:?}");
            assert!(a.contains(DragAction::MOVE), "falta mover en {modo:?}");
        }
    }

    #[test]
    fn no_se_ofrece_una_accion_que_no_sabemos_cumplir() {
        // `LINK` haría que el destino cree un enlace simbólico esperando que el
        // origen colabore, y no hay nada de eso implementado.
        let a = acciones_de(DragMode::Copy);
        assert!(!a.contains(DragAction::LINK));
        assert!(!a.contains(DragAction::ASK));
    }

    #[test]
    fn la_accion_informada_es_la_que_negocio_el_destino() {
        assert_eq!(nombre_de_accion(DragAction::COPY), Some("copy"));
        assert_eq!(nombre_de_accion(DragAction::MOVE), Some("move"));
        assert_eq!(nombre_de_accion(DragAction::empty()), None);
    }

    #[test]
    fn con_las_dos_puestas_se_informa_la_mas_fuerte() {
        // No debería pasar —el destino elige una— pero informar «copy» cuando hubo
        // un movimiento dejaría el original donde estaba sin que nada lo dijera.
        assert_eq!(
            nombre_de_accion(DragAction::COPY | DragAction::MOVE),
            Some("move")
        );
    }

    #[test]
    fn una_accion_que_no_ofrecemos_no_se_informa_como_nuestra() {
        // `LINK` y `ASK` no se ofrecen; si llegaran, no son ni copiar ni mover.
        assert_eq!(nombre_de_accion(DragAction::LINK), None);
        assert_eq!(nombre_de_accion(DragAction::ASK), None);
    }
}
