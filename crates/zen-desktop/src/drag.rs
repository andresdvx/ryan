//! Detectar que el usuario está arrastrando algo (por ejemplo un archivo desde el Explorador) en cualquier parte del
//! sistema, antes de que llegue a una ventana de Ryan. Windows no avisa de esto: se mira si el botón izquierdo del
//! ratón está pulsado y si existe la ventana de la imagen que acompaña al cursor durante un arrastre del shell
//! (`SysDragImage`, la que usan el Explorador, el escritorio y los navegadores). Comprobado a mano con un arrastre
//! real desde el Explorador: la ventana aparece unas décimas de segundo después de empezar y desaparece al soltar
//! o cancelar. `FindWindow` no la encuentra, pero enumerar las ventanas sí.

use std::ffi::c_void;

use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::System::Ole::RegisterDragDrop;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows_sys::Win32::UI::WindowsAndMessaging::{EnumChildWindows, EnumWindows, GetClassNameW, GetPropW, IsWindowVisible};

/// Clase de la ventana con la imagen del arrastre (el icono o la miniatura que sigue al cursor).
const DRAG_IMAGE_CLASS: &str = "SysDragImage";

struct Search<'a> {
    class: &'a str,
    found: bool,
}

unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` es el puntero a la `Search` que `window_class_exists` mantiene viva durante la enumeración.
    let search = unsafe { &mut *(lparam as *mut Search) };
    let mut buffer = [0u16; 128];
    // SAFETY: el búfer es propio y se pasa con su longitud real.
    let length = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    // SAFETY: `hwnd` lo entrega la propia enumeración.
    let visible = length > 0 && unsafe { IsWindowVisible(hwnd) } != 0;
    if visible && String::from_utf16_lossy(&buffer[..length as usize]).eq_ignore_ascii_case(search.class) {
        search.found = true;
        return 0; // ya está: se deja de enumerar
    }
    1
}

/// ¿Hay alguna ventana visible de esta clase?
fn window_class_exists(class: &str) -> bool {
    let mut search = Search { class, found: false };
    // SAFETY: la enumeración es síncrona: `search` vive más que la llamada y el callback solo la usa dentro de ella.
    unsafe {
        EnumWindows(Some(visit), &mut search as *mut Search as LPARAM);
    }
    search.found
}

/// ¿Hay un arrastre en curso con el botón izquierdo del ratón?
pub fn drag_in_progress() -> bool {
    // SAFETY: `GetAsyncKeyState` no recibe punteros.
    let pressed = unsafe { (GetAsyncKeyState(VK_LBUTTON as i32) as u16) & 0x8000 != 0 };
    // Solo se enumeran ventanas con el botón pulsado: el resto del tiempo esto cuesta una sola llamada.
    pressed && window_class_exists(DRAG_IMAGE_CLASS)
}

// ---------------------------------------------------------------------------------------------
// Reparar el destino de arrastre de una ventana de Tauri
// ---------------------------------------------------------------------------------------------

/// Propiedad de ventana donde Windows guarda el destino de arrastre registrado con `RegisterDragDrop`.
const DROP_TARGET_PROP: &str = "OleDropTargetInterface";

struct Children {
    prop: Vec<u16>,
    /// Cada ventana interna con su clase y el destino de arrastre que tiene (nulo si no tiene).
    found: Vec<(HWND, String, *mut c_void)>,
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` es el puntero a la `Children` que `repair_drop_targets` mantiene viva durante la enumeración.
    let list = unsafe { &mut *(lparam as *mut Children) };
    let mut buffer = [0u16; 128];
    // SAFETY: el búfer es propio y se pasa con su longitud real; `prop` termina en cero.
    let (length, target) = unsafe { (GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32), GetPropW(hwnd, list.prop.as_ptr())) };
    let class = if length > 0 { String::from_utf16_lossy(&buffer[..length as usize]) } else { String::new() };
    list.found.push((hwnd, class, target));
    1
}

/// Tauri registra en Windows el destino de arrastre de una ventana UNA vez, al crearla, sobre las ventanas internas de
/// WebView2 que existan en ese instante. Si WebView2 aún no las ha creado (pasa con las ventanas que se crean al final,
/// con el equipo ocupado arrancando), el destino queda a medias y esa ventana no recibe archivos arrastrados. Esto
/// completa el registro: copia el destino que ya tiene a las ventanas internas que no lo tienen. Debe llamarse desde
/// el hilo de la interfaz. Devuelve cuántas ventanas internas se repararon.
pub fn repair_drop_targets(top: *mut c_void) -> usize {
    let mut list = Children { prop: DROP_TARGET_PROP.encode_utf16().chain(std::iter::once(0)).collect(), found: Vec::new() };
    // SAFETY: la enumeración es síncrona: `list` vive más que la llamada y el callback solo la usa dentro de ella.
    unsafe {
        EnumChildWindows(top, Some(collect), &mut list as *mut Children as LPARAM);
    }
    let Some(template) = list.found.iter().map(|(_, _, target)| *target).find(|target| !target.is_null()) else { return 0 };
    let mut repaired = 0;
    for (hwnd, class, target) in &list.found {
        // `WRY_WEBVIEW` es el contenedor de Tauri: nunca llevó destino y no hace falta.
        if target.is_null() && class != "WRY_WEBVIEW" {
            // SAFETY: `hwnd` salió de la enumeración y `template` es un `IDropTarget` vivo que registró Tauri.
            if unsafe { RegisterDragDrop(*hwnd, template) } >= 0 {
                repaired += 1;
            }
        }
    }
    repaired
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_arrastre_no_hay_falsos_positivos() {
        // En una prueba automática nadie arrastra nada.
        assert!(!drag_in_progress());
        assert!(!window_class_exists("ClaseQueNoExiste_12345"));
    }

    #[test]
    fn la_enumeracion_encuentra_ventanas_que_si_existen() {
        // La barra de tareas de Windows siempre existe en una sesión con escritorio; sin sesión (un servidor de
        // pruebas) no se puede comprobar y no se considera un fallo.
        if !window_class_exists("Shell_TrayWnd") {
            eprintln!("prueba omitida: no hay barra de tareas en esta sesión");
            return;
        }
        assert!(window_class_exists("shell_traywnd"), "la clase se compara sin distinguir mayúsculas");
    }
}
