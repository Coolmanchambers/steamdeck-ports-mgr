//! The QR code on the sign-in page: the sign-in address as dark and light squares, and a picture of them.
//!
//! The encoding is the `qrcode` crate (MIT or Apache-2.0, no other dependency without its default
//! features); this file only turns its answer into pixels the page window can show.

use qrcode::{Color, EcLevel, QrCode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrMatrix {
    /// Squares per side.
    pub size: usize,
    /// Row by row; true is dark.
    pub dark: Vec<bool>,
}

impl QrMatrix {
    /// `None` when the text does not fit (it always fits for an address).
    pub fn encode(text: &str) -> Option<QrMatrix> {
        let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).ok()?;
        let size = code.width();
        let dark = code.to_colors().into_iter().map(|c| c == Color::Dark).collect::<Vec<_>>();
        (dark.len() == size * size).then_some(QrMatrix { size, dark })
    }

    fn is_dark(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.dark[y * self.size + x]
    }

    /// RGBA pixels with a white border of `quiet` squares (scanners need one) and `scale` pixels per square:
    /// `(width, height, bytes)`.
    pub fn rgba(&self, scale: usize, quiet: usize) -> (u32, u32, Vec<u8>) {
        let side = (self.size + 2 * quiet) * scale;
        let mut out = vec![255u8; side * side * 4];
        for y in 0..side {
            for x in 0..side {
                let (mx, my) = (x / scale, y / scale);
                let dark = mx >= quiet && my >= quiet && self.is_dark(mx - quiet, my - quiet);
                if dark {
                    let at = (y * side + x) * 4;
                    out[at..at + 3].copy_from_slice(&[0, 0, 0]);
                }
            }
        }
        (side as u32, side as u32, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finder_at(m: &QrMatrix, x0: usize, y0: usize) -> bool {
        // 7x7: dark ring, light ring, 3x3 dark centre.
        (0..7).all(|dy| {
            (0..7).all(|dx| {
                let ring = dx.min(6 - dx).min(dy.min(6 - dy));
                m.is_dark(x0 + dx, y0 + dy) == (ring == 0 || ring >= 2)
            })
        })
    }

    #[test]
    fn an_address_makes_a_square_code_with_the_three_finder_squares() {
        let m = QrMatrix::encode("https://www.google.com/device").unwrap();
        assert!(m.size >= 21 && m.size <= 41 && (m.size - 17) % 4 == 0, "{}", m.size);
        assert_eq!(m.dark.len(), m.size * m.size);
        assert!(finder_at(&m, 0, 0) && finder_at(&m, m.size - 7, 0) && finder_at(&m, 0, m.size - 7));
    }

    #[test]
    fn the_same_text_gives_the_same_code_and_other_text_another() {
        assert_eq!(QrMatrix::encode("https://www.google.com/device"), QrMatrix::encode("https://www.google.com/device"));
        assert_ne!(QrMatrix::encode("https://www.google.com/device"), QrMatrix::encode("https://example.com/other"));
    }

    #[test]
    fn the_picture_has_a_white_border_and_the_right_size() {
        let m = QrMatrix::encode("https://www.google.com/device").unwrap();
        let (w, h, px) = m.rgba(4, 4);
        assert_eq!((w as usize, h as usize), ((m.size + 8) * 4, (m.size + 8) * 4));
        assert_eq!(px.len(), (w * h * 4) as usize);
        assert_eq!(&px[0..4], &[255, 255, 255, 255], "the corner is white");
        // The top-left finder square starts at (4,4) squares: its first pixel is black.
        let at = ((4 * 4) * w as usize + 4 * 4) * 4;
        assert_eq!(&px[at..at + 4], &[0, 0, 0, 255]);
        assert!(px.chunks(4).all(|p| p[3] == 255), "fully opaque");
    }
}
