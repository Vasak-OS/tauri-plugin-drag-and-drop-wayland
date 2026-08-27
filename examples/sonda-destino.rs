//! Sonda 3: un destino que dice exactamente qué recibe.
//!
//! Interpretar lo que hace una app ajena con un drop es adivinar: puede recibir
//! todo bien y decidir mostrarlo distinto. Esta ventana acepta el arrastre y
//! imprime, para cada tipo MIME, los bytes tal cual llegaron.
//!
//! `cargo run --example sonda-destino` y arrastrale un archivo.

use gtk::prelude::*;

fn main() {
    gtk::init().expect("gtk init");

    // Con `--mover` acepta **sólo** `MOVE`, así el destino negocia mover y GTK le
    // pide al origen que borre el original. Es la única forma de probar ese camino
    // sin depender de que alguna app ajena elija mover.
    let solo_mover = std::env::args().any(|a| a == "--mover");

    let ventana = gtk::Window::new(gtk::WindowType::Toplevel);
    ventana.set_title(if solo_mover {
        "Soltá acá — este destino MUEVE (borra el original)"
    } else {
        "Soltá un archivo acá — sonda de destino"
    });
    ventana.set_default_size(520, 200);

    let etiqueta = gtk::Label::new(Some("Soltá un archivo acá.\nLo que reciba se imprime en la terminal."));
    etiqueta.set_line_wrap(true);
    let caja = gtk::EventBox::new();
    caja.add(&etiqueta);
    ventana.add(&caja);

    // Se aceptan los tres tipos que ofrece el plugin, para ver cuál elige GTK y
    // qué contenido llega en cada uno.
    let objetivos = [
        gtk::TargetEntry::new("text/uri-list", gtk::TargetFlags::OTHER_APP, 0),
        gtk::TargetEntry::new("text/plain", gtk::TargetFlags::OTHER_APP, 1),
        gtk::TargetEntry::new("text/plain;charset=utf-8", gtk::TargetFlags::OTHER_APP, 2),
    ];
    // **Sin `DROP`.** Con `DestDefaults::ALL`, GTK cierra el arrastre solo llamando
    // a `gtk_drag_finish(context, TRUE, FALSE, time)` — con el borrado en `FALSE`.
    // O sea que `drag-data-delete` nunca llega al origen y un movimiento nunca
    // termina de moverse. Medido: el destino negociaba `MOVE`, recibía los datos, y
    // el archivo seguía en su lugar. Un destino que mueve de verdad tiene que
    // cerrarlo él, que es lo que hace Nautilus.
    caja.drag_dest_set(
        gtk::DestDefaults::MOTION | gtk::DestDefaults::HIGHLIGHT,
        &objetivos,
        if solo_mover {
            gtk::gdk::DragAction::MOVE
        } else {
            gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE
        },
    );

    // Sin `DROP` hay que pedir los datos a mano cuando se suelta.
    caja.connect_drag_drop(|widget, contexto, _, _, tiempo| {
        let objetivo = widget
            .drag_dest_find_target(contexto, None)
            .unwrap_or_else(|| gtk::gdk::Atom::intern("text/uri-list"));
        println!("soltado: se piden los datos de {}", objetivo.name());
        widget.drag_get_data(contexto, &objetivo, tiempo);
        // `drag-drop` devuelve un bool: `true` = «me hago cargo yo».
        true
    });

    caja.connect_drag_data_received(move |_, contexto, _, _, datos, info, tiempo| {
        let mime = datos.data_type().name();
        println!();
        println!("── recibido ──");
        println!("  objetivo: {info} ({mime})");
        println!("  acción elegida: {:?}", contexto.selected_action());

        let bytes = datos.data();
        println!("  bytes: {}", bytes.len());
        println!("  como texto: {:?}", String::from_utf8_lossy(&bytes));

        // Y lo que de verdad importa: ¿se pueden abrir los archivos que dice?
        let uris = datos.uris();
        if uris.is_empty() {
            println!("  URIs: ninguno (este objetivo no trae archivos)");
        } else {
            for u in uris {
                match gtk::glib::filename_from_uri(&u) {
                    Ok((ruta, _)) => {
                        let existe = ruta.is_file();
                        let tamano = std::fs::metadata(&ruta).map(|m| m.len()).unwrap_or(0);
                        println!(
                            "  URI: {u}\n    ruta: {}\n    ¿existe?: {existe}  bytes: {tamano}",
                            ruta.display()
                        );
                        if existe {
                            println!("    ENTREGA CORRECTA: el archivo se puede abrir");
                        } else {
                            println!("    FALLA: la ruta no corresponde a un archivo");
                        }
                    }
                    Err(e) => println!("  URI: {u}\n    FALLA: no se pudo convertir a ruta: {e}"),
                }
            }
        }

        // El cierre, a mano. El tercer argumento es el que le pide al origen que
        // borre el original: sin él en `true`, un movimiento se queda a medias.
        let mover = contexto.selected_action().contains(gtk::gdk::DragAction::MOVE);
        println!("  cerrando: éxito=true borrar_origen={mover}");
        contexto.drag_finish(true, mover, tiempo);
    });

    ventana.connect_delete_event(|_, _| {
        gtk::main_quit();
        gtk::glib::Propagation::Proceed
    });

    ventana.show_all();
    if solo_mover {
        println!("destino listo (SÓLO MOVER): lo que sueltes se le pide borrar al origen");
    } else {
        println!("destino listo: arrastrale un archivo desde el gestor");
    }
    gtk::main();
}
