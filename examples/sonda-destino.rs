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

    let ventana = gtk::Window::new(gtk::WindowType::Toplevel);
    ventana.set_title("Soltá un archivo acá — sonda de destino");
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
    caja.drag_dest_set(
        gtk::DestDefaults::ALL,
        &objetivos,
        gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
    );

    caja.connect_drag_data_received(|_, contexto, _, _, datos, info, _| {
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
    });

    ventana.connect_delete_event(|_, _| {
        gtk::main_quit();
        gtk::glib::Propagation::Proceed
    });

    ventana.show_all();
    println!("destino listo: arrastrale un archivo desde el gestor");
    gtk::main();
}
