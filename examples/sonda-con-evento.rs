//! Sonda 2: ¿alcanza con pasarle el evento del botón?
//!
//! La sonda 1 midió que sin evento `drag_begin_with_coordinates` devuelve `None`
//! en Wayland, con cualquier widget. Esta comprueba la solución: guardar el evento
//! del botón en un manejador y pasárselo después.
//!
//! Necesita **una interacción**: apretar el botón izquierdo dentro de la ventana y
//! arrastrar unos píxeles sin soltar. Eso es exactamente lo que hace alguien
//! arrastrando un archivo.
//!
//! `cargo run --example sonda-con-evento`

use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

fn main() {
    gtk::init().expect("gtk init");

    let ventana = gtk::Window::new(gtk::WindowType::Toplevel);
    ventana.set_title("Sonda de arrastre — apretá y arrastrá acá dentro");
    ventana.set_default_size(420, 160);

    let etiqueta = gtk::Label::new(Some(
        "Apretá el botón izquierdo acá dentro\ny arrastrá unos píxeles sin soltar.",
    ));
    let caja = gtk::EventBox::new();
    caja.add(&etiqueta);
    ventana.add(&caja);

    // El evento del botón, guardado igual que tendría que hacerlo el plugin.
    let ultimo_press: Rc<RefCell<Option<gtk::gdk::Event>>> = Rc::new(RefCell::new(None));
    let ya_probado = Rc::new(RefCell::new(false));

    caja.add_events(
        gtk::gdk::EventMask::BUTTON_PRESS_MASK
            | gtk::gdk::EventMask::BUTTON_RELEASE_MASK
            | gtk::gdk::EventMask::POINTER_MOTION_MASK,
    );

    {
        let ultimo_press = ultimo_press.clone();
        caja.connect_button_press_event(move |_, evento| {
            println!("botón apretado (tipo {:?})", evento.event_type());
            // `EventButton` no hace `upcast`; el evento genérico se toma del
            // manejador, que es lo mismo que tendría que guardar el plugin.
            *ultimo_press.borrow_mut() = gtk::current_event();
            gtk::glib::Propagation::Proceed
        });
    }

    {
        let ultimo_press = ultimo_press.clone();
        let ya_probado = ya_probado.clone();
        caja.connect_motion_notify_event(move |widget, evento| {
            if *ya_probado.borrow() {
                return gtk::glib::Propagation::Proceed;
            }
            // Sólo con el botón apretado, como el gesto de verdad.
            if !evento.state().contains(gtk::gdk::ModifierType::BUTTON1_MASK) {
                return gtk::glib::Propagation::Proceed;
            }
            *ya_probado.borrow_mut() = true;

            let lista = gtk::TargetList::new(&[]);
            lista.add(&gtk::gdk::Atom::intern("text/uri-list"), 0, 0);
            let w = widget.clone().upcast::<gtk::Widget>();
            w.drag_source_set(
                gtk::gdk::ModifierType::BUTTON1_MASK,
                &[],
                gtk::gdk::DragAction::COPY,
            );
            w.drag_source_set_target_list(Some(&lista));

            println!();
            println!("── A) sin evento, como hace el plugin hoy ──");
            match w.drag_begin_with_coordinates(&lista, gtk::gdk::DragAction::COPY, 1, None, -1, -1)
            {
                Some(c) => println!("   contexto: protocolo {:?}", c.protocol()),
                None => println!("   None — no empieza"),
            }

            println!("── B) con el evento del botón guardado ──");
            let guardado = ultimo_press.borrow().clone();
            match guardado {
                None => println!("   no había evento guardado"),
                Some(ev) => {
                    match w.drag_begin_with_coordinates(
                        &lista,
                        gtk::gdk::DragAction::COPY,
                        1,
                        Some(&ev),
                        -1,
                        -1,
                    ) {
                        Some(c) => {
                            println!("   contexto: protocolo {:?}", c.protocol());
                            println!("   ACCIÓN: el arrastre arrancó. Soltá el botón.");
                        }
                        None => println!("   None — tampoco empieza con evento"),
                    }
                }
            }

            println!("── C) con el evento actual del manejador ──");
            match gtk::current_event() {
                None => println!("   no hay evento actual"),
                Some(ev) => match w.drag_begin_with_coordinates(
                    &lista,
                    gtk::gdk::DragAction::COPY,
                    1,
                    Some(&ev),
                    -1,
                    -1,
                ) {
                    Some(c) => println!("   contexto: protocolo {:?}", c.protocol()),
                    None => println!("   None"),
                },
            }

            println!();
            println!("sonda terminada — cerrá la ventana");
            gtk::glib::Propagation::Proceed
        });
    }

    ventana.connect_delete_event(|_, _| {
        gtk::main_quit();
        gtk::glib::Propagation::Proceed
    });

    ventana.show_all();
    println!("ventana abierta: apretá y arrastrá dentro de ella");
    gtk::main();
}
