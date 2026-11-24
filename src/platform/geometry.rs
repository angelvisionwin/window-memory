//! Rectángulos en píxeles físicos de pantalla.

use windows::Win32::Foundation::RECT;

/// Lado mínimo para considerar que un rectángulo describe una ventana real.
///
/// Descarta barras flotantes, indicadores y ventanas residuales de pocos píxeles.
pub const MIN_SIDE: i32 = 80;

/// Rectángulo en píxeles físicos, con el mismo formato que `RECT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn from_rect(rect: RECT) -> Rect {
        Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }

    /// Conversión inversa; solo se usa en las pruebas.
    #[cfg(test)]
    pub fn to_rect(self) -> RECT {
        RECT {
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom,
        }
    }

    /// Anchura; nunca negativa, aunque el rectángulo esté invertido.
    pub fn width(self) -> i32 {
        (self.right - self.left).max(0)
    }

    /// Altura; nunca negativa, aunque el rectángulo esté invertido.
    pub fn height(self) -> i32 {
        (self.bottom - self.top).max(0)
    }

    /// `true` si el rectángulo tiene un tamaño utilizable como geometría.
    pub fn is_usable(self) -> bool {
        self.width() >= MIN_SIDE && self.height() >= MIN_SIDE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_never_goes_negative() {
        let inverted = Rect::new(100, 100, 20, 40);
        assert_eq!(inverted.width(), 0);
        assert_eq!(inverted.height(), 0);
        assert!(!inverted.is_usable());
    }

    #[test]
    fn usability_needs_a_minimum_side() {
        assert!(Rect::new(0, 0, MIN_SIDE, MIN_SIDE).is_usable());
        assert!(!Rect::new(0, 0, MIN_SIDE - 1, 1000).is_usable());
        assert!(!Rect::new(0, 0, 1000, MIN_SIDE - 1).is_usable());
    }

    #[test]
    fn conversion_with_rect_round_trips() {
        let rect = Rect::new(-1920, 0, 0, 1080);
        assert_eq!(Rect::from_rect(rect.to_rect()), rect);
    }
}
