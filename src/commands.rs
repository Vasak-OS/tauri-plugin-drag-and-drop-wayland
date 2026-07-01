use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gtk::prelude::*;
use gtk::gdk::DragAction;
use log::{info, warn, error};
use tauri::{command, AppHandle, Runtime, Window};
use tauri::ipc::Channel;

use crate::error::Error;
use crate::models::*;

thread_local! {
    static WIDGET_CACHE: RefCell<HashMap<String, gtk::Widget>> = RefCell::new(HashMap::new());
}

const URI_TARGET_ID: u32 = 0;
const TEXT_TARGET_ID: u32 = 1;

fn find_webview_widget(window: &gtk::ApplicationWindow, window_label: &str) -> Option<gtk::Widget> {
    if let Some(cached) = WIDGET_CACHE.with(|cache| cache.borrow().get(window_label).cloned()) {
        if cached.is_visible() {
            return Some(cached);
        }
    }
    let found = find_widget_by_type_name(&window.clone().upcast::<gtk::Container>(), "WebKitWebView");
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

fn load_pixbuf_from_path(path: &PathBuf) -> Option<gdk_pixbuf::Pixbuf> {
    let data = std::fs::read(path).ok()?;
    load_pixbuf_from_data(&data)
}

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
    let window_id = window.label().to_string();
    let files = match item {
        DragItem::Files(paths) => Some(paths),
        DragItem::Data { .. } => None,
    };

    let (tx, rx) = std::sync::mpsc::channel();

    app.run_on_main_thread(move || {
        let icon_pixbuf = image.as_ref().and_then(|img| match img {
            Image::Base64(data) => {
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)
                    .ok()
                    .and_then(|bytes| load_pixbuf_from_data(&bytes))
            }
            Image::Raw(path) => {
                let p = PathBuf::from(path);
                load_pixbuf_from_path(&p)
            }
        });
        let result = match window.gtk_window() {
            Ok(w) => {
                let drag_widget = find_webview_widget(&w, &window_id).unwrap_or_else(|| {
                    warn!("WebKitWebView not found, falling back to ApplicationWindow");
                    w.upcast::<gtk::Widget>()
                });
                info!("Starting drag on widget: {:?}", drag_widget.type_().name());
                do_drag(&drag_widget, files, icon_pixbuf, opts.mode, on_event)
            }
            Err(e) => {
                error!("Failed to get GTK window: {e}");
                Err(Error::Tauri(e))
            }
        };
        let _ = tx.send(result);
    }).map_err(Error::Tauri)?;

    rx.recv().map_err(|e| Error::Drag(format!("drag result channel closed: {e}")))?
}

fn do_drag(
    widget: &gtk::Widget,
    file_paths: Option<Vec<PathBuf>>,
    icon: Option<gdk_pixbuf::Pixbuf>,
    mode: DragMode,
    on_event: Channel<CallbackResult>,
) -> Result<()> {
    let drag_action = match mode {
        DragMode::Copy => DragAction::COPY,
        DragMode::Move => DragAction::MOVE,
    };

    widget.drag_source_set(gtk::gdk::ModifierType::BUTTON1_MASK, &[], drag_action);

    let target_list = gtk::TargetList::new(&[]);

    if file_paths.is_some() {
        target_list.add(&gtk::gdk::Atom::intern("text/uri-list"), 0, URI_TARGET_ID);
    }
    target_list.add(&gtk::gdk::Atom::intern("text/plain"), 0, TEXT_TARGET_ID);
    target_list.add(&gtk::gdk::Atom::intern("text/plain;charset=utf-8"), 0, TEXT_TARGET_ID);

    widget.drag_source_set_target_list(Some(&target_list));

    let handler_ids: Arc<Mutex<Vec<gtk::glib::SignalHandlerId>>> =
        Arc::new(Mutex::new(vec![]));

    let paths = file_paths.unwrap_or_default();
    let h = widget.connect_drag_data_get(move |_, _, data, info, _| {
        if paths.is_empty() {
            return;
        }
        let uris: Vec<String> =
            paths.iter().map(|p| format!("file://{}", p.display())).collect();
        match info {
            URI_TARGET_ID => {
                info!("Drag target requested: text/uri-list");
                let uris_refs: Vec<&str> = uris.iter().map(String::as_str).collect();
                data.set_uris(&uris_refs);
            }
            TEXT_TARGET_ID => {
                info!("Drag target requested: text/plain");
                data.set_text(&uris.join("\r\n"));
            }
            _ => {
                info!("Drag target requested: unknown({})", info);
            }
        }
    });
    handler_ids.lock().unwrap().push(h);

    info!("Drag targets configured");

    let drag_context = widget
        .drag_begin_with_coordinates(&target_list, drag_action, 1, None, -1, -1)
        .ok_or_else(|| {
            Error::Drag("failed to initiate drag".into())
        })?;

    info!("Drag context created");

    if let Some(pixbuf) = icon {
        drag_context.drag_set_icon_pixbuf(&pixbuf, 0, 0);
    }

    let w = widget.clone();
    let ids = handler_ids.clone();
    let ch_drop = on_event.clone();
    drag_context.connect_drop_performed(move |_, _| {
        info!("Drag drop performed");
        let _ = ch_drop.send(CallbackResult {
            result: DragResult::Dropped,
            cursor_pos: CursorPosition { x: 0.0, y: 0.0 },
        });
        cleanup(&w, &ids);
    });

    let w = widget.clone();
    let ids = handler_ids.clone();
    widget.connect_drag_failed(move |_, _, _| {
        warn!("Drag failed or was cancelled");
        let _ = on_event.send(CallbackResult {
            result: DragResult::Cancelled,
            cursor_pos: CursorPosition { x: 0.0, y: 0.0 },
        });
        cleanup(&w, &ids);
        gtk::glib::Propagation::Proceed
    });

    info!("GDK drag initiated successfully");
    Ok(())
}

fn cleanup(
    widget: &gtk::Widget,
    handler_ids: &Arc<Mutex<Vec<gtk::glib::SignalHandlerId>>>,
) {
    let mut ids = handler_ids.lock().unwrap();
    for id in ids.drain(..) {
        widget.disconnect(id);
    }
    widget.drag_source_unset();
}
