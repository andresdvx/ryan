//! Botones y campos de otras ventanas a través de «UI Automation», la API de accesibilidad de Windows.
//! Así se pulsa un control por su nombre («Guardar», «Aceptar») sin adivinar coordenadas de pantalla.

use windows::core::Interface;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern,
    IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern, TreeScope_Descendants, UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId,
    UIA_ComboBoxControlTypeId, UIA_CONTROLTYPE_ID, UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_ExpandCollapsePatternId,
    UIA_HyperlinkControlTypeId, UIA_InvokePatternId, UIA_ListItemControlTypeId, UIA_MenuItemControlTypeId, UIA_RadioButtonControlTypeId,
    UIA_SelectionItemPatternId, UIA_SplitButtonControlTypeId, UIA_TabItemControlTypeId, UIA_TogglePatternId, UIA_TreeItemControlTypeId,
};

/// Se recorren como mucho estos elementos de una ventana (los navegadores tienen miles).
const MAX_SCANNED: i32 = 3_000;
/// Y se devuelven como mucho estos controles.
pub const MAX_LISTED: usize = 150;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    pub name: String,
    pub kind: &'static str,
}

#[allow(non_upper_case_globals)] // las constantes de Windows se llaman así
fn kind_of(id: UIA_CONTROLTYPE_ID) -> Option<&'static str> {
    Some(match id {
        UIA_ButtonControlTypeId | UIA_SplitButtonControlTypeId => "botón",
        UIA_CheckBoxControlTypeId => "casilla",
        UIA_RadioButtonControlTypeId => "opción",
        UIA_MenuItemControlTypeId => "menú",
        UIA_TabItemControlTypeId => "pestaña",
        UIA_HyperlinkControlTypeId => "enlace",
        UIA_ListItemControlTypeId | UIA_TreeItemControlTypeId => "elemento",
        UIA_EditControlTypeId | UIA_DocumentControlTypeId => "campo de texto",
        UIA_ComboBoxControlTypeId => "lista desplegable",
        _ => return None,
    })
}

/// Ejecuta `work` en un hilo propio con COM inicializado: así no depende del estado de COM del hilo que llame.
fn on_com_thread<T: Send + 'static>(work: impl FnOnce(&IUIAutomation) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    std::thread::spawn(move || {
        // SAFETY: inicializa COM para este hilo; se hace una sola vez y el hilo termina después.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        // SAFETY: crea el objeto estándar de UI Automation.
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.map_err(|e| format!("no se pudo iniciar UI Automation: {e}"))?;
        work(&automation)
    })
    .join()
    .map_err(|_| "UI Automation falló inesperadamente".to_string())?
}

struct Found {
    element: IUIAutomationElement,
    control: Control,
}

fn scan(automation: &IUIAutomation, hwnd: isize) -> Result<Vec<Found>, String> {
    let fail = |e: windows::core::Error| format!("no se pudo leer los controles de la ventana: {e}");
    // SAFETY: llamadas COM sobre objetos recién creados; los HWND inválidos devuelven error.
    unsafe {
        let root = automation.ElementFromHandle(HWND(hwnd as *mut _)).map_err(fail)?;
        let all = root.FindAll(TreeScope_Descendants, &automation.CreateTrueCondition().map_err(fail)?).map_err(fail)?;
        let count = all.Length().map_err(fail)?.min(MAX_SCANNED);
        let mut found = Vec::new();
        for index in 0..count {
            let Ok(element) = all.GetElement(index) else { continue };
            let Ok(kind_id) = element.CurrentControlType() else { continue };
            let Some(kind) = kind_of(kind_id) else { continue };
            if element.CurrentIsEnabled().map(|b| !b.as_bool()).unwrap_or(true) || element.CurrentIsOffscreen().map(|b| b.as_bool()).unwrap_or(true) {
                continue;
            }
            let name = element.CurrentName().map(|n| n.to_string()).unwrap_or_default();
            let name = name.trim().replace(['\r', '\n'], " ");
            // Un campo de texto o un documento suele no tener nombre y aun así se puede enfocar.
            if name.is_empty() && kind != "campo de texto" {
                continue;
            }
            found.push(Found { element, control: Control { name: name.chars().take(80).collect(), kind } });
        }
        Ok(found)
    }
}

/// Todos los controles útiles de la ventana en el orden en que aparecen (sin quitar repetidos ni recortar).
pub fn scan_controls(hwnd: isize) -> Result<Vec<Control>, String> {
    on_com_thread(move |automation| Ok(scan(automation, hwnd)?.into_iter().map(|f| f.control).collect()))
}

/// Pulsa el control que se llama exactamente `name`; `nth` elige entre varios con el mismo nombre (0 = el primero).
pub fn activate_nth(hwnd: isize, name: &str, nth: usize) -> Result<Control, String> {
    let name = name.to_string();
    on_com_thread(move |automation| {
        let found = scan(automation, hwnd)?;
        let target = found
            .iter()
            .filter(|f| f.control.name == name)
            .nth(nth)
            .ok_or_else(|| format!("«{name}» ya no está en la ventana."))?;
        press(target)
    })
}

/// Controles visibles y activos de la ventana (botones, casillas, pestañas, campos…).
pub fn list_controls(hwnd: isize) -> Result<Vec<Control>, String> {
    on_com_thread(move |automation| {
        let mut controls: Vec<Control> = scan(automation, hwnd)?.into_iter().map(|f| f.control).collect();
        controls.dedup();
        controls.truncate(MAX_LISTED);
        Ok(controls)
    })
}

/// Elige el control por su nombre: primero coincidencia exacta, luego parcial. Si hay varios, error.
fn pick<'a>(found: &'a [Found], name: &str) -> Result<&'a Found, String> {
    let wanted = name.trim().to_lowercase();
    let exact: Vec<&Found> = found.iter().filter(|f| f.control.name.to_lowercase() == wanted).collect();
    let candidates = if exact.is_empty() { found.iter().filter(|f| f.control.name.to_lowercase().contains(&wanted)).collect() } else { exact };
    match candidates.as_slice() {
        [] => Err(format!("No encontré ningún control llamado «{name}» en esa ventana.")),
        [one] => Ok(one),
        many => {
            let names: Vec<String> = many.iter().take(8).map(|f| format!("«{}» ({})", f.control.name, f.control.kind)).collect();
            Err(format!("Hay {} controles que coinciden con «{name}»: {}. Sé más específico.", many.len(), names.join(", ")))
        }
    }
}

/// Descripción del control que se pulsaría, sin pulsarlo (para la confirmación).
pub fn describe(hwnd: isize, name: &str) -> Result<Control, String> {
    let name = name.to_string();
    on_com_thread(move |automation| pick(&scan(automation, hwnd)?, &name).map(|f| f.control.clone()))
}

/// Pulsa el control: invoca, marca/desmarca, selecciona, despliega o, en un campo de texto, le da el foco.
pub fn activate(hwnd: isize, name: &str) -> Result<Control, String> {
    let name = name.to_string();
    on_com_thread(move |automation| {
        let found = scan(automation, hwnd)?;
        press(pick(&found, &name)?)
    })
}

/// Invoca, marca, selecciona, despliega o da el foco al control, según lo que admita.
fn press(target: &Found) -> Result<Control, String> {
    {
        let element = &target.element;
        let fail = |e: windows::core::Error| format!("no se pudo pulsar «{}»: {e}", target.control.name);
        // SAFETY: llamadas COM sobre un elemento vivo; cada patrón se pide y se convierte a su interfaz.
        unsafe {
            if let Ok(pattern) = element.GetCurrentPattern(UIA_InvokePatternId).and_then(|p| p.cast::<IUIAutomationInvokePattern>()) {
                pattern.Invoke().map_err(fail)?;
            } else if let Ok(pattern) = element.GetCurrentPattern(UIA_TogglePatternId).and_then(|p| p.cast::<IUIAutomationTogglePattern>()) {
                pattern.Toggle().map_err(fail)?;
            } else if let Ok(pattern) = element.GetCurrentPattern(UIA_SelectionItemPatternId).and_then(|p| p.cast::<IUIAutomationSelectionItemPattern>()) {
                pattern.Select().map_err(fail)?;
            } else if let Ok(pattern) = element.GetCurrentPattern(UIA_ExpandCollapsePatternId).and_then(|p| p.cast::<IUIAutomationExpandCollapsePattern>()) {
                pattern.Expand().map_err(fail)?;
            } else {
                element.SetFocus().map_err(fail)?;
            }
        }
        Ok(target.control.clone())
    }
}
