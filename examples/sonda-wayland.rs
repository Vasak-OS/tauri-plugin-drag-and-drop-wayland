//! Sonda: ¿qué hace `drag_begin_with_coordinates` sin un evento?
//!
//! El plugin arranca el arrastre desde un comando de IPC, o sea desde una
//! devolución de llamada del bucle principal y no desde un manejador de eventos.
//! Ahí `gtk_get_current_event()` devuelve nulo, y en Wayland
//! `wl_data_device.start_drag` necesita el serial de un agarre implícito del ratón.
//!
//! Esta sonda distingue los dos fallos posibles, que se arreglan distinto:
//!
//!   * GTK devuelve `None` → el arrastre nunca empieza y el plugin informa
//!     «failed to initiate drag».
//!   * GTK devuelve un contexto pero el compositor descarta la petición → el
//!     arrastre «empieza» sin que pase nada y no llega ningún callback.
//!
//! Se corre a mano: `cargo run --example sonda-wayland`.

use gtk::prelude::*;

fn main() {
    if let Err(e) = gtk::init() {
        eprintln!("no se pudo inicializar GTK: {e}");
        std::process::exit(1);
    }

    let backend = gtk::gdk::Display::default()
        .expect("display")
        .type_()
        .name()
        .to_string();
    println!("backend de GDK: {backend}");

    let ventana = gtk::Window::new(gtk::WindowType::Toplevel);
    ventana.set_default_size(1, 1);
    // Fuera de la vista: esto es una sonda, no algo para mirar.
    ventana.set_decorated(false);
    ventana.set_opacity(0.0);
    // Tres candidatos, para separar «falta el evento» de «el widget no tiene
    // ventana GDK propia»: una caja (no tiene), un `EventBox` (sí tiene) y la
    // ventana misma (sí tiene, y es una ventana Wayland de verdad).
    let caja = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let event_box = gtk::EventBox::new();
    event_box.add(&caja);
    ventana.add(&event_box);
    ventana.show_all();

    gtk::glib::idle_add_local_once({
        let ventana = ventana.clone();
        let event_box = event_box.clone();
        let caja = caja.clone();
        move || {
            println!("¿hay evento actual?: {}", gtk::current_event().is_some());
            println!();

            let candidatos: Vec<(&str, gtk::Widget)> = vec![
                ("GtkBox (sin ventana GDK propia)", caja.clone().upcast()),
                ("GtkEventBox (con ventana GDK)", event_box.clone().upcast()),
                ("GtkWindow (toplevel Wayland)", ventana.clone().upcast()),
            ];

            for (nombre, widget) in candidatos {
                let lista = gtk::TargetList::new(&[]);
                lista.add(&gtk::gdk::Atom::intern("text/uri-list"), 0, 0);
                widget.drag_source_set(
                    gtk::gdk::ModifierType::BUTTON1_MASK,
                    &[],
                    gtk::gdk::DragAction::COPY,
                );
                widget.drag_source_set_target_list(Some(&lista));

                let tiene_ventana = widget.window().is_some();
                let r = widget.drag_begin_with_coordinates(
                    &lista,
                    gtk::gdk::DragAction::COPY,
                    1,
                    None,
                    -1,
                    -1,
                );
                println!(
                    "{nombre}\n  ventana GDK: {tiene_ventana}\n  drag_begin: {}",
                    match &r {
                        Some(c) => format!("contexto (protocolo {:?})", c.protocol()),
                        None => "None — no empieza".to_string(),
                    }
                );
                println!();
            }

            gtk::main_quit();
        }
    });

    gtk::glib::timeout_add_seconds_local_once(5, || {
        println!("RESULTADO: se agotó el tiempo");
        gtk::main_quit();
    });

    gtk::main();
    println!("sonda terminada");
}
