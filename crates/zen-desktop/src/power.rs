//! Estado de la batería y decisión de cuándo avisar. La lectura es una llamada a Windows; la decisión
//! (`BatteryWatcher`) es lógica pura y se prueba con lecturas inventadas.

use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

/// Avisa por debajo de este porcentaje, y otra vez por debajo del crítico.
pub const LOW_PERCENT: u8 = 20;
pub const CRITICAL_PERCENT: u8 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    /// Enchufado a la corriente.
    pub plugged_in: bool,
}

/// `None` en equipos sin batería o si Windows no sabe el nivel.
pub fn read_battery() -> Option<Battery> {
    // SAFETY: estructura propia que la API rellena.
    let status = unsafe {
        let mut status: SYSTEM_POWER_STATUS = std::mem::zeroed();
        if GetSystemPowerStatus(&mut status) == 0 {
            return None;
        }
        status
    };
    // BatteryFlag: 128 = sin batería, 255 = desconocido. BatteryLifePercent: 255 = desconocido.
    if status.BatteryFlag & 128 != 0 || status.BatteryFlag == 255 || status.BatteryLifePercent > 100 {
        return None;
    }
    Some(Battery { percent: status.BatteryLifePercent, plugged_in: status.ACLineStatus == 1 })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryNotice {
    Low(u8),
    Charged,
}

/// Recuerda qué se avisó ya para no repetirlo en cada lectura.
#[derive(Debug, Default)]
pub struct BatteryWatcher {
    warned_low: bool,
    warned_critical: bool,
    announced_charged: bool,
}

impl BatteryWatcher {
    pub fn observe(&mut self, battery: Battery) -> Option<BatteryNotice> {
        if battery.plugged_in {
            // Al enchufar, se vuelve a poder avisar la próxima vez que se agote.
            self.warned_low = false;
            self.warned_critical = false;
            if battery.percent >= 100 {
                if !self.announced_charged {
                    self.announced_charged = true;
                    return Some(BatteryNotice::Charged);
                }
            } else if battery.percent < 95 {
                self.announced_charged = false;
            }
            return None;
        }
        self.announced_charged = false;
        if battery.percent <= CRITICAL_PERCENT && !self.warned_critical {
            self.warned_critical = true;
            self.warned_low = true;
            return Some(BatteryNotice::Low(battery.percent));
        }
        if battery.percent <= LOW_PERCENT && battery.percent > CRITICAL_PERCENT && !self.warned_low {
            self.warned_low = true;
            return Some(BatteryNotice::Low(battery.percent));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_battery(percent: u8) -> Battery {
        Battery { percent, plugged_in: false }
    }
    fn plugged(percent: u8) -> Battery {
        Battery { percent, plugged_in: true }
    }

    #[test]
    fn avisa_una_vez_al_bajar_del_20_y_otra_al_bajar_del_10() {
        let mut watcher = BatteryWatcher::default();
        assert_eq!(watcher.observe(on_battery(60)), None);
        assert_eq!(watcher.observe(on_battery(21)), None);
        assert_eq!(watcher.observe(on_battery(20)), Some(BatteryNotice::Low(20)));
        assert_eq!(watcher.observe(on_battery(19)), None, "no repite");
        assert_eq!(watcher.observe(on_battery(11)), None);
        assert_eq!(watcher.observe(on_battery(10)), Some(BatteryNotice::Low(10)));
        assert_eq!(watcher.observe(on_battery(4)), None, "no repite el crítico");
    }

    #[test]
    fn si_arranca_ya_muy_baja_avisa_una_sola_vez() {
        let mut watcher = BatteryWatcher::default();
        assert_eq!(watcher.observe(on_battery(7)), Some(BatteryNotice::Low(7)));
        assert_eq!(watcher.observe(on_battery(6)), None);
        assert_eq!(watcher.observe(on_battery(5)), None);
    }

    #[test]
    fn enchufada_no_avisa_de_nivel_bajo_y_rearma_los_avisos() {
        let mut watcher = BatteryWatcher::default();
        assert_eq!(watcher.observe(plugged(8)), None);
        assert_eq!(watcher.observe(on_battery(15)), Some(BatteryNotice::Low(15)));
        assert_eq!(watcher.observe(plugged(16)), None);
        assert_eq!(watcher.observe(on_battery(19)), Some(BatteryNotice::Low(19)), "tras enchufar vuelve a avisar");
    }

    #[test]
    fn avisa_la_carga_completa_una_vez_por_carga() {
        let mut watcher = BatteryWatcher::default();
        assert_eq!(watcher.observe(plugged(98)), None);
        assert_eq!(watcher.observe(plugged(100)), Some(BatteryNotice::Charged));
        assert_eq!(watcher.observe(plugged(100)), None);
        assert_eq!(watcher.observe(on_battery(99)), None);
        assert_eq!(watcher.observe(plugged(100)), Some(BatteryNotice::Charged), "tras usar la batería, otra carga");
    }

    #[test]
    fn lectura_real_no_falla() {
        // Puede ser None en un equipo de sobremesa; lo importante es que no entre en pánico.
        if let Some(battery) = read_battery() {
            assert!(battery.percent <= 100);
        }
    }
}
