//! Geometría del paseo de la mascota por el borde inferior del escritorio (coordenadas en píxeles).

/// Posiciones horizontales permitidas para el borde izquierdo de la ventana.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkBounds {
    pub left: i32,
    pub right: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facing {
    Left,
    Right,
}

impl Facing {
    pub fn from_direction(direction: i32) -> Self {
        if direction < 0 {
            Facing::Left
        } else {
            Facing::Right
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Facing::Left => "left",
            Facing::Right => "right",
        }
    }
}

/// Dirección real: si la pedida choca contra el borde, la mascota da la vuelta en lugar de quedarse quieta.
pub fn effective_direction(x: i32, wanted: i32, bounds: WalkBounds) -> i32 {
    if wanted < 0 && x <= bounds.left {
        1
    } else if wanted >= 0 && x >= bounds.right {
        -1
    } else if wanted < 0 {
        -1
    } else {
        1
    }
}

/// Posición tras recorrer `travelled` píxeles desde `start_x`, y si chocó con un borde.
pub fn position_after(start_x: i32, direction: i32, travelled: i32, bounds: WalkBounds) -> (i32, bool) {
    let wanted = start_x + direction * travelled;
    let x = wanted.clamp(bounds.left, bounds.right.max(bounds.left));
    (x, x != wanted)
}

/// Hacia dónde mira la mascota respecto al cursor. `None` si está lejos o casi justo encima
/// (donde girar de un lado a otro con cada pequeño movimiento sería molesto).
pub fn facing_towards(dx: f64, dy: f64, radius: f64, dead_zone: f64) -> Option<Facing> {
    if dx.hypot(dy) > radius || dx.abs() < dead_zone {
        return None;
    }
    Some(if dx < 0.0 { Facing::Left } else { Facing::Right })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDS: WalkBounds = WalkBounds { left: 10, right: 500 };

    #[test]
    fn da_la_vuelta_si_choca_contra_el_borde() {
        assert_eq!(effective_direction(10, -1, BOUNDS), 1);
        assert_eq!(effective_direction(500, 1, BOUNDS), -1);
        assert_eq!(effective_direction(11, -1, BOUNDS), -1);
        assert_eq!(effective_direction(499, 1, BOUNDS), 1);
        assert_eq!(effective_direction(250, -1, BOUNDS), -1);
        assert_eq!(effective_direction(250, 1, BOUNDS), 1);
    }

    #[test]
    fn avanza_sin_pasarse_de_los_bordes() {
        assert_eq!(position_after(100, 1, 50, BOUNDS), (150, false));
        assert_eq!(position_after(100, -1, 50, BOUNDS), (50, false));
        assert_eq!(position_after(480, 1, 50, BOUNDS), (500, true));
        assert_eq!(position_after(30, -1, 50, BOUNDS), (10, true));
        assert_eq!(position_after(100, 1, 0, BOUNDS), (100, false));
    }

    #[test]
    fn una_pantalla_diminuta_no_hace_entrar_en_panico() {
        let tiny = WalkBounds { left: 10, right: 5 };
        assert_eq!(position_after(7, 1, 20, tiny), (10, true));
    }

    #[test]
    fn mira_hacia_el_cursor_solo_si_esta_cerca_y_no_encima() {
        assert_eq!(facing_towards(-100.0, 0.0, 300.0, 20.0), Some(Facing::Left));
        assert_eq!(facing_towards(100.0, 50.0, 300.0, 20.0), Some(Facing::Right));
        assert_eq!(facing_towards(10.0, 0.0, 300.0, 20.0), None, "demasiado encima");
        assert_eq!(facing_towards(400.0, 0.0, 300.0, 20.0), None, "demasiado lejos");
        assert_eq!(facing_towards(250.0, 250.0, 300.0, 20.0), None, "la distancia es diagonal");
    }

    #[test]
    fn la_direccion_se_traduce_a_orientacion() {
        assert_eq!(Facing::from_direction(-1), Facing::Left);
        assert_eq!(Facing::from_direction(1), Facing::Right);
        assert_eq!((Facing::Left.as_str(), Facing::Right.as_str()), ("left", "right"));
    }
}