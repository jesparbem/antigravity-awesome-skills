// En Windows, una compilación de release no debe abrir una consola detrás de la
// ventana de la aplicación.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    factoria_desktop_lib::run()
}
