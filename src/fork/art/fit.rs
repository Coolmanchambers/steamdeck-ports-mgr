//! Fitting any source image into the exact shape of a Steam art slot. Pure functions, no files, no
//! network.
//!
//! | Slot | Output | How |
//! |---|---|---|
//! | Portrait | exactly 600x900 | contain, centered, over an enlarged blurred copy of the same image |
//! | Wide | exactly 920x430 | cover, cropped from the center |
//! | Hero | exactly 1920x620 | cover, cropped from the center |
//! | Logo | fits inside 1280x720 | scaled down if larger, never cropped, never enlarged, alpha kept |
//! | Icon | exactly 512x512 | center crop when nearly square, otherwise padded with transparency |
//!
//! A source that would have to be enlarged by more than [`MAX_UPSCALE`] is refused with
//! [`FitError::TooSmall`]: a soft picture in a Steam slot looks worse than the slot's fallback. The
//! blurred backdrop behind a portrait is not checked, it is meant to be soft.
//!
//! Portrait, wide and hero come out opaque (transparency is flattened over black). Logo and icon keep
//! their alpha. Resizing is done on premultiplied color so a transparent edge does not bleed dark
//! fringes into the picture.

use std::fmt;
use std::io::Cursor;

use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageFormat, ImageReader, Limits, Rgba, RgbaImage};

use crate::fork::artwork::ArtworkKind;

/// Largest enlargement accepted ("about 1.5x").
pub const MAX_UPSCALE: f64 = 1.5;
/// Sources bigger than this per side are not decoded (decompression-bomb guard).
const MAX_SOURCE_SIDE: u32 = 4096;
const MAX_DECODE_ALLOC: u64 = 192 * 1024 * 1024;
/// Icons up to this far from square are cropped square; wilder shapes are padded.
const ICON_CROP_MAX_RATIO: f64 = 1.2;
/// Smallest square icon [`fit_icon_keep_size`] accepts.
pub const ICON_MIN_SIDE: u32 = 32;
/// A logo whose longer side is below this is refused as too small.
pub const LOGO_MIN_SIDE: u32 = 64;
/// Portrait sources whose contained picture would fill less than this share of the slot are refused.
const PORTRAIT_MIN_FILL: f64 = 0.2;

pub const PORTRAIT_SIZE: (u32, u32) = (600, 900);
pub const WIDE_SIZE: (u32, u32) = (920, 430);
pub const HERO_SIZE: (u32, u32) = (1920, 620);
/// Bounding box for the logo, not an exact size.
pub const LOGO_MAX: (u32, u32) = (1280, 720);
pub const ICON_SIZE: (u32, u32) = (512, 512);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    Portrait,
    Wide,
    Hero,
    Logo,
    Icon,
}

impl Slot {
    pub const ALL: [Slot; 5] = [Slot::Portrait, Slot::Wide, Slot::Hero, Slot::Logo, Slot::Icon];

    /// Exact output size; for the logo, the box it must fit inside.
    pub fn size(self) -> (u32, u32) {
        match self {
            Slot::Portrait => PORTRAIT_SIZE,
            Slot::Wide => WIDE_SIZE,
            Slot::Hero => HERO_SIZE,
            Slot::Logo => LOGO_MAX,
            Slot::Icon => ICON_SIZE,
        }
    }

    pub fn kind(self) -> ArtworkKind {
        match self {
            Slot::Portrait => ArtworkKind::Cover,
            Slot::Wide => ArtworkKind::Wide,
            Slot::Hero => ArtworkKind::Hero,
            Slot::Logo => ArtworkKind::Logo,
            Slot::Icon => ArtworkKind::Icon,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Slot::Portrait => "portrait",
            Slot::Wide => "wide",
            Slot::Hero => "hero",
            Slot::Logo => "logo",
            Slot::Icon => "icon",
        }
    }
}

#[derive(Debug)]
pub enum FitError {
    /// The file could not be read.
    Read(String),
    /// The bytes are not a PNG, JPEG or WebP the decoder accepts (or are over the size guard).
    Decode(String),
    /// Zero width or height.
    Empty,
    /// The source would have to be enlarged by `scale` (> [`MAX_UPSCALE`]) to fill the slot.
    TooSmall { slot: Slot, width: u32, height: u32, scale: f64 },
    /// The source is so far from the slot's shape that contain-and-pad would leave a sliver of art
    /// in a sea of blur (a banner for the portrait slot).
    UnsuitableShape { slot: Slot, width: u32, height: u32 },
    Encode(String),
}

impl fmt::Display for FitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FitError::Read(e) => write!(f, "could not read the image: {e}"),
            FitError::Decode(e) => write!(f, "could not decode the image: {e}"),
            FitError::Empty => write!(f, "the image has no pixels"),
            FitError::TooSmall { slot: Slot::Logo, width, height, .. } => {
                write!(f, "{width}x{height} is too small for a logo (the longer side must be at least {LOGO_MIN_SIDE} px)")
            }
            FitError::TooSmall { slot, width, height, scale } => write!(
                f,
                "{width}x{height} is too small for the {} slot (would need {scale:.2}x enlargement, limit {MAX_UPSCALE}x)",
                slot.name()
            ),
            FitError::UnsuitableShape { slot, width, height } => {
                write!(f, "{width}x{height} has the wrong shape for the {} slot", slot.name())
            }
            FitError::Encode(e) => write!(f, "could not encode the image: {e}"),
        }
    }
}

impl std::error::Error for FitError {}

/// A slot image ready to be written, with where it came from for the provenance manifest.
#[derive(Debug, Clone)]
pub struct Fitted {
    pub slot: Slot,
    pub image: RgbaImage,
    /// Free text recorded in the manifest, e.g. `catalog`, `steamgriddb`.
    pub source: String,
    /// True for art this app composed itself (padded, cropped from other art), as opposed to a source
    /// image delivered in the slot's own shape.
    pub generated: bool,
}

impl Fitted {
    pub fn with_source(mut self, source: &str) -> Fitted {
        self.source = source.to_string();
        self
    }

    /// True when the image has the size its slot requires: exact for portrait, wide, hero; inside
    /// 1280x720 (and at least [`LOGO_MIN_SIDE`] long) for the logo; square between [`ICON_MIN_SIDE`]
    /// and 512 for the icon. The grid writer refuses anything else, since the fields are public.
    pub fn fits_slot(&self) -> bool {
        let (w, h) = self.image.dimensions();
        match self.slot {
            Slot::Portrait | Slot::Wide | Slot::Hero => (w, h) == self.slot.size(),
            Slot::Logo => w >= 1 && h >= 1 && w <= LOGO_MAX.0 && h <= LOGO_MAX.1 && w.max(h) >= LOGO_MIN_SIDE,
            Slot::Icon => w == h && (ICON_MIN_SIDE..=ICON_SIZE.0).contains(&w),
        }
    }

    pub fn to_png(&self) -> Result<Vec<u8>, FitError> {
        let mut out = Vec::new();
        self.image.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).map_err(|e| FitError::Encode(e.to_string()))?;
        Ok(out)
    }
}

/// Decodes PNG, JPEG or WebP bytes with a size guard (4096 px per side covers 4K art). EXIF rotation
/// is not applied: a sideways phone photo stays sideways.
pub fn decode(bytes: &[u8]) -> Result<DynamicImage, FitError> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| FitError::Decode(e.to_string()))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    reader.decode().map_err(|e| FitError::Decode(e.to_string()))
}

/// Decodes `bytes` and fits them into `slot`.
pub fn fit_bytes(slot: Slot, bytes: &[u8]) -> Result<Fitted, FitError> {
    fit(slot, &decode(bytes)?)
}

/// Fits `source` into `slot`. See the module docs for each slot's rule.
pub fn fit(slot: Slot, source: &DynamicImage) -> Result<Fitted, FitError> {
    let rgba = source.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return Err(FitError::Empty);
    }
    let (image, generated) = match slot {
        Slot::Portrait => portrait(slot, &rgba)?,
        Slot::Wide | Slot::Hero => cover(slot, &rgba)?,
        Slot::Logo => (logo(slot, &rgba)?, false),
        Slot::Icon => icon(slot, &rgba)?,
    };
    Ok(Fitted { slot, image, source: String::new(), generated })
}

/// Icon for sources that are already square: kept at their own size when between
/// [`ICON_MIN_SIDE`] and 512, since the icon is shown small and a 256 px catalog icon enlarged to
/// 512 would be refused by [`fit`]. Anything else goes through [`fit`] (so 1024 is shrunk to 512,
/// a 16 px or non-square source is refused or padded as `fit` decides).
pub fn fit_icon_keep_size(source: &DynamicImage) -> Result<Fitted, FitError> {
    let rgba = source.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == h && (ICON_MIN_SIDE..=ICON_SIZE.0).contains(&w) {
        return Ok(Fitted { slot: Slot::Icon, image: rgba, source: String::new(), generated: false });
    }
    fit(Slot::Icon, source)
}

/// How much a `w` x `h` source would have to be enlarged to fill `slot` (1.0 or less: none). A source
/// whose answer is above [`MAX_UPSCALE`] is refused by [`fit`]; callers use this to skip such a
/// candidate before downloading it. A logo has no size to reach, so only a tiny one counts as "too
/// small" (reported as a large factor).
pub fn upscale_needed(slot: Slot, w: u32, h: u32) -> f64 {
    if w == 0 || h == 0 {
        return f64::INFINITY;
    }
    let (tw, th) = (f64::from(slot.size().0), f64::from(slot.size().1));
    let (w, h) = (f64::from(w), f64::from(h));
    match slot {
        Slot::Portrait => f64::min(tw / w, th / h),
        Slot::Wide | Slot::Hero => f64::max(tw / w, th / h),
        Slot::Logo => {
            if w.max(h) < f64::from(LOGO_MIN_SIDE) {
                f64::from(LOGO_MIN_SIDE) / w.max(h)
            } else {
                1.0
            }
        }
        Slot::Icon => {
            let (long, short) = (w.max(h), w.min(h));
            if long / short <= ICON_CROP_MAX_RATIO {
                tw / short
            } else {
                tw / long
            }
        }
    }
}

fn check_scale(slot: Slot, w: u32, h: u32, scale: f64) -> Result<(), FitError> {
    if scale > MAX_UPSCALE + 1e-9 {
        Err(FitError::TooSmall { slot, width: w, height: h, scale })
    } else {
        Ok(())
    }
}

/// Scaled size, at least 1 px per side.
fn scaled(w: u32, h: u32, scale: f64) -> (u32, u32) {
    (((f64::from(w) * scale).round() as u32).max(1), ((f64::from(h) * scale).round() as u32).max(1))
}

/// True when any pixel is not fully opaque.
pub fn has_transparency(img: &RgbaImage) -> bool {
    img.pixels().any(|p| p[3] < 255)
}

/// Resize that does not smear the color of fully transparent pixels into their neighbors.
fn resize(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    if img.dimensions() == (w, h) {
        return img.clone();
    }
    if !has_transparency(img) {
        return imageops::resize(img, w, h, FilterType::Lanczos3);
    }
    let mut pre = img.clone();
    for p in pre.pixels_mut() {
        let a = u32::from(p[3]);
        for c in 0..3 {
            p[c] = ((u32::from(p[c]) * a + 127) / 255) as u8;
        }
    }
    let mut out = imageops::resize(&pre, w, h, FilterType::Lanczos3);
    for p in out.pixels_mut() {
        let a = u32::from(p[3]);
        if a == 0 {
            *p = Rgba([0, 0, 0, 0]);
            continue;
        }
        for c in 0..3 {
            p[c] = ((u32::from(p[c]) * 255 + a / 2) / a).min(255) as u8;
        }
    }
    out
}

/// Replaces transparency by black, giving an opaque image.
fn flatten_over_black(img: &mut RgbaImage) {
    for p in img.pixels_mut() {
        let a = u32::from(p[3]);
        if a != 255 {
            for c in 0..3 {
                p[c] = ((u32::from(p[c]) * a + 127) / 255) as u8;
            }
            p[3] = 255;
        }
    }
}

/// Scale to fill `target` completely and crop the overflow equally from both sides. No limit on
/// enlargement.
fn cover_unchecked(img: &RgbaImage, target: (u32, u32)) -> RgbaImage {
    let (w, h) = img.dimensions();
    let scale = f64::max(f64::from(target.0) / f64::from(w), f64::from(target.1) / f64::from(h));
    let (nw, nh) = scaled(w, h, scale);
    let (nw, nh) = (nw.max(target.0), nh.max(target.1));
    let big = resize(img, nw, nh);
    imageops::crop_imm(&big, (nw - target.0) / 2, (nh - target.1) / 2, target.0, target.1).to_image()
}

fn cover(slot: Slot, img: &RgbaImage) -> Result<(RgbaImage, bool), FitError> {
    let (w, h) = img.dimensions();
    let target = slot.size();
    let scale = f64::max(f64::from(target.0) / f64::from(w), f64::from(target.1) / f64::from(h));
    check_scale(slot, w, h, scale)?;
    let mut out = cover_unchecked(img, target);
    flatten_over_black(&mut out);
    // A cover-crop of a source that already had the slot's shape is the source itself.
    let same_shape = (f64::from(w) * f64::from(target.1) - f64::from(h) * f64::from(target.0)).abs() < f64::from(h.max(w));
    Ok((out, !same_shape))
}

fn portrait(slot: Slot, img: &RgbaImage) -> Result<(RgbaImage, bool), FitError> {
    let (w, h) = img.dimensions();
    let (tw, th) = PORTRAIT_SIZE;
    let scale = f64::min(f64::from(tw) / f64::from(w), f64::from(th) / f64::from(h));
    check_scale(slot, w, h, scale)?;
    let (mut fw, mut fh) = scaled(w, h, scale);
    if f64::from(fw) * f64::from(fh) < PORTRAIT_MIN_FILL * f64::from(tw) * f64::from(th) {
        return Err(FitError::UnsuitableShape { slot, width: w, height: h });
    }
    // The limiting side lands exactly on the slot; rounding must not push the other past it.
    fw = fw.min(tw);
    fh = fh.min(th);
    if f64::from(tw) / f64::from(w) <= f64::from(th) / f64::from(h) {
        fw = tw;
    } else {
        fh = th;
    }
    let front = resize(img, fw, fh);
    if (fw, fh) == (tw, th) {
        let mut out = front;
        flatten_over_black(&mut out);
        return Ok((out, false));
    }
    let mut back = backdrop(img, PORTRAIT_SIZE);
    imageops::overlay(&mut back, &front, i64::from((tw - fw) / 2), i64::from((th - fh) / 2));
    flatten_over_black(&mut back);
    Ok((back, true))
}

/// A soft, enlarged copy of the image that fills `target`: cover-fit at a quarter of the size, blur,
/// scale back up (cheap, and the blur hides the enlargement).
fn backdrop(img: &RgbaImage, target: (u32, u32)) -> RgbaImage {
    let mut flat = img.clone();
    flatten_over_black(&mut flat);
    let small_target = ((target.0 / 4).max(1), (target.1 / 4).max(1));
    let small = cover_unchecked(&flat, small_target);
    let blurred = imageops::blur(&small, 4.0);
    imageops::resize(&blurred, target.0, target.1, FilterType::Triangle)
}

fn logo(slot: Slot, img: &RgbaImage) -> Result<RgbaImage, FitError> {
    let (w, h) = img.dimensions();
    if w.max(h) < LOGO_MIN_SIDE {
        return Err(FitError::TooSmall { slot, width: w, height: h, scale: f64::from(LOGO_MIN_SIDE) / f64::from(w.max(h)) });
    }
    let scale = f64::min(f64::from(LOGO_MAX.0) / f64::from(w), f64::from(LOGO_MAX.1) / f64::from(h));
    if scale >= 1.0 {
        return Ok(img.clone());
    }
    let (nw, nh) = scaled(w, h, scale);
    Ok(resize(img, nw.min(LOGO_MAX.0), nh.min(LOGO_MAX.1)))
}

fn icon(slot: Slot, img: &RgbaImage) -> Result<(RgbaImage, bool), FitError> {
    let (w, h) = img.dimensions();
    let (long, short) = (w.max(h), w.min(h));
    let side = ICON_SIZE.0;
    if f64::from(long) / f64::from(short) <= ICON_CROP_MAX_RATIO {
        let scale = f64::from(side) / f64::from(short);
        check_scale(slot, w, h, scale)?;
        let square = imageops::crop_imm(img, (w - short) / 2, (h - short) / 2, short, short).to_image();
        return Ok((resize(&square, side, side), long != short));
    }
    let scale = f64::from(side) / f64::from(long);
    check_scale(slot, w, h, scale)?;
    let (nw, nh) = scaled(w, h, scale);
    let (nw, nh) = (nw.min(side), nh.min(side));
    let small = resize(img, nw, nh);
    let mut canvas = RgbaImage::from_pixel(side, side, Rgba([0, 0, 0, 0]));
    imageops::replace(&mut canvas, &small, i64::from((side - nw) / 2), i64::from((side - nh) / 2));
    Ok((canvas, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(w, h, Rgba(c)))
    }

    fn dims(f: &Fitted) -> (u32, u32) {
        f.image.dimensions()
    }

    #[test]
    fn upscale_needed_matches_what_fit_accepts() {
        for (slot, w, h) in [
            (Slot::Portrait, 399, 598),
            (Slot::Portrait, 400, 600),
            (Slot::Hero, 1280, 413),
            (Slot::Hero, 1920, 620),
            (Slot::Wide, 600, 280),
            (Slot::Logo, 40, 40),
            (Slot::Logo, 64, 64),
            (Slot::Icon, 300, 300),
            (Slot::Icon, 600, 900),
        ] {
            let accepted = fit(slot, &solid(w, h, [9, 9, 9, 255])).is_ok();
            let predicted = upscale_needed(slot, w, h) <= MAX_UPSCALE + 1e-9;
            assert_eq!(accepted, predicted, "{slot:?} {w}x{h}");
        }
        assert!(upscale_needed(Slot::Hero, 0, 5).is_infinite());
    }

    #[test]
    fn golden_800x600_box_gives_exactly_600x900() {
        let out = fit(Slot::Portrait, &solid(800, 600, [200, 30, 30, 255])).unwrap();
        assert_eq!(dims(&out), (600, 900));
        assert!(out.generated);
        assert!(out.image.pixels().all(|p| p[3] == 255));
    }

    #[test]
    fn landscape_box_is_contained_not_cropped_and_padded_with_a_soft_copy() {
        // Gray box with a red marker flush with its left edge. Cropping to 2:3 would lose the marker.
        let mut src = RgbaImage::from_pixel(800, 600, Rgba([128, 128, 128, 255]));
        for y in 280..320 {
            for x in 0..40 {
                src.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        let out = fit(Slot::Portrait, &DynamicImage::ImageRgba8(src)).unwrap().image;
        let marker = out.get_pixel(5, 450);
        assert!(marker[0] > 200 && marker[1] < 60, "marker lost: {marker:?}");
        // The padding is a blurred copy of the art (gray here), not black and not the marker.
        let pad = out.get_pixel(300, 10);
        assert!((100..=156).contains(&pad[0]) && (100..=156).contains(&pad[1]), "padding is {pad:?}");
        assert!(out.get_pixel(5, 10)[1] > 60, "marker leaked into the padding");
    }

    #[test]
    fn portrait_already_2_to_3_is_not_padded() {
        let out = fit(Slot::Portrait, &solid(600, 900, [10, 20, 30, 255])).unwrap();
        assert_eq!(dims(&out), (600, 900));
        assert!(!out.generated);
        assert_eq!(*out.image.get_pixel(0, 0), Rgba([10, 20, 30, 255]));
    }

    #[test]
    fn tall_source_is_contained_by_width_and_padded_top_and_bottom_or_sides() {
        // 1:2 is narrower than 2:3, so it is padded left and right.
        let out = fit(Slot::Portrait, &solid(500, 1000, [0, 200, 0, 255])).unwrap();
        assert_eq!(dims(&out), (600, 900));
    }

    #[test]
    fn golden_1920x1080_gives_exactly_1920x620_and_920x430() {
        let src = solid(1920, 1080, [1, 2, 3, 255]);
        assert_eq!(dims(&fit(Slot::Hero, &src).unwrap()), (1920, 620));
        assert_eq!(dims(&fit(Slot::Wide, &src).unwrap()), (920, 430));
    }

    #[test]
    fn cover_crop_keeps_the_vertical_center() {
        // Bands of 230 / 620 / 230 rows: a 1920x620 center crop is exactly the middle band.
        let mut src = RgbaImage::new(1920, 1080);
        for (_, y, p) in src.enumerate_pixels_mut() {
            *p = match y {
                0..=229 => Rgba([255, 0, 0, 255]),
                230..=849 => Rgba([0, 255, 0, 255]),
                _ => Rgba([0, 0, 255, 255]),
            };
        }
        let src = DynamicImage::ImageRgba8(src);
        let hero = fit(Slot::Hero, &src).unwrap().image;
        for y in [0, 310, 619] {
            assert_eq!(*hero.get_pixel(960, y), Rgba([0, 255, 0, 255]), "row {y}");
        }
        let wide = fit(Slot::Wide, &src).unwrap().image;
        assert_eq!(*wide.get_pixel(460, 215), Rgba([0, 255, 0, 255]));
        // Scaled to 518 rows the red/green boundary is at row ~110; a centered 430-row crop starts at row 44,
        // so the boundary sits at output row ~66. A top-anchored crop puts it at 110, a bottom-anchored one
        // at 22, so rows 55 and 80 tell the three apart.
        assert!(wide.get_pixel(460, 55)[0] > 200, "row 55 should still be the red band");
        assert!(wide.get_pixel(460, 80)[1] > 200, "row 80 should already be the green band");
    }

    #[test]
    fn source_already_in_slot_shape_is_not_marked_generated() {
        assert!(!fit(Slot::Hero, &solid(1920, 620, [9, 9, 9, 255])).unwrap().generated);
        assert!(fit(Slot::Hero, &solid(1920, 1080, [9, 9, 9, 255])).unwrap().generated);
    }

    #[test]
    fn wide_and_hero_come_out_opaque() {
        let out = fit(Slot::Hero, &solid(1920, 1080, [200, 200, 200, 100])).unwrap();
        assert!(out.image.pixels().all(|p| p[3] == 255));
    }

    #[test]
    fn logo_keeps_its_alpha_and_is_never_cropped() {
        // Opaque center, transparent frame.
        let mut src = RgbaImage::from_pixel(2000, 1000, Rgba([0, 0, 0, 0]));
        for y in 250..750 {
            for x in 500..1500 {
                src.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let out = fit(Slot::Logo, &DynamicImage::ImageRgba8(src)).unwrap();
        assert_eq!(dims(&out), (1280, 640));
        assert_eq!(out.image.get_pixel(0, 0)[3], 0);
        assert_eq!(*out.image.get_pixel(640, 320), Rgba([255, 255, 255, 255]));
        assert!(!out.generated);
    }

    #[test]
    fn logo_edge_pixels_keep_their_color_instead_of_fringing_dark() {
        // Left half opaque white, right half fully transparent with black color data.
        let mut src = RgbaImage::from_pixel(2000, 1000, Rgba([0, 0, 0, 0]));
        for y in 0..1000 {
            for x in 0..1000 {
                src.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let out = fit(Slot::Logo, &DynamicImage::ImageRgba8(src)).unwrap().image;
        let partial: Vec<_> = out.pixels().filter(|p| p[3] > 0 && p[3] < 255).collect();
        assert!(!partial.is_empty());
        assert!(partial.iter().all(|p| p[0] >= 250 && p[1] >= 250 && p[2] >= 250), "dark fringe on partial alpha");
    }

    #[test]
    fn small_logo_is_left_alone_and_not_enlarged() {
        let out = fit(Slot::Logo, &solid(300, 100, [255, 255, 255, 128])).unwrap();
        assert_eq!(dims(&out), (300, 100));
        assert_eq!(out.image.get_pixel(5, 5)[3], 128);
    }

    #[test]
    fn tall_logo_fits_inside_the_box() {
        let out = fit(Slot::Logo, &solid(500, 1800, [255, 255, 255, 255])).unwrap();
        assert_eq!(dims(&out), (200, 720));
    }

    #[test]
    fn icon_downscales_to_512() {
        let out = fit(Slot::Icon, &solid(1024, 1024, [5, 6, 7, 255])).unwrap();
        assert_eq!(dims(&out), (512, 512));
        assert!(!out.generated);
    }

    #[test]
    fn nearly_square_icon_is_cropped_and_stays_opaque() {
        let out = fit(Slot::Icon, &solid(600, 500, [5, 6, 7, 255])).unwrap();
        assert_eq!(dims(&out), (512, 512));
        assert!(out.image.pixels().all(|p| p[3] == 255));
    }

    #[test]
    fn wide_icon_source_is_padded_with_transparency() {
        let out = fit(Slot::Icon, &solid(1000, 500, [5, 6, 7, 255])).unwrap();
        assert_eq!(dims(&out), (512, 512));
        assert_eq!(out.image.get_pixel(256, 5)[3], 0);
        assert_eq!(out.image.get_pixel(256, 256)[3], 255);
        assert!(out.generated);
    }

    #[test]
    fn upscaling_past_one_and_a_half_is_refused_with_a_typed_error() {
        match fit(Slot::Portrait, &solid(300, 450, [0, 0, 0, 255])) {
            Err(FitError::TooSmall { slot: Slot::Portrait, width: 300, height: 450, scale }) => assert!((scale - 2.0).abs() < 1e-9),
            other => panic!("expected TooSmall, got {other:?}"),
        }
        assert!(matches!(fit(Slot::Hero, &solid(640, 360, [0, 0, 0, 255])), Err(FitError::TooSmall { .. })));
        assert!(matches!(fit(Slot::Wide, &solid(400, 200, [0, 0, 0, 255])), Err(FitError::TooSmall { .. })));
        assert!(matches!(fit(Slot::Icon, &solid(256, 256, [0, 0, 0, 255])), Err(FitError::TooSmall { .. })));
    }

    #[test]
    fn exactly_one_and_a_half_times_is_still_accepted() {
        assert_eq!(dims(&fit(Slot::Portrait, &solid(400, 600, [0, 0, 0, 255])).unwrap()), (600, 900));
        assert_eq!(dims(&fit(Slot::Hero, &solid(1280, 720, [0, 0, 0, 255])).unwrap()), (1920, 620));
        assert!(matches!(fit(Slot::Portrait, &solid(399, 598, [0, 0, 0, 255])), Err(FitError::TooSmall { .. })));
    }

    #[test]
    fn every_accepted_source_gives_the_exact_slot_size() {
        let sizes = [(600, 900), (450, 600), (1000, 1000), (1920, 1080), (1081, 1919), (700, 1200)];
        let mut accepted = 0;
        for (w, h) in sizes {
            let src = solid(w, h, [90, 90, 90, 255]);
            for slot in [Slot::Portrait, Slot::Wide, Slot::Hero, Slot::Icon] {
                match fit(slot, &src) {
                    Ok(out) => {
                        accepted += 1;
                        assert_eq!(dims(&out), slot.size(), "{w}x{h} into {}", slot.name());
                    }
                    Err(FitError::TooSmall { .. }) => {}
                    Err(e) => panic!("{w}x{h} into {}: {e}", slot.name()),
                }
            }
        }
        assert!(accepted >= 12, "only {accepted} of 24 combinations were accepted");
    }

    #[test]
    fn keep_size_icon_accepts_small_square_catalog_icons() {
        let out = fit_icon_keep_size(&solid(256, 256, [1, 1, 1, 255])).unwrap();
        assert_eq!(dims(&out), (256, 256));
        assert_eq!(dims(&fit_icon_keep_size(&solid(48, 48, [1, 1, 1, 255])).unwrap()), (48, 48));
        assert_eq!(dims(&fit_icon_keep_size(&solid(1024, 1024, [1, 1, 1, 255])).unwrap()), (512, 512));
        assert!(matches!(fit_icon_keep_size(&solid(16, 16, [1, 1, 1, 255])), Err(FitError::TooSmall { .. })));
        assert!(matches!(fit_icon_keep_size(&solid(200, 100, [1, 1, 1, 255])), Err(FitError::TooSmall { .. })));
    }

    #[test]
    fn fit_bytes_decodes_png_and_reports_garbage() {
        let png = Fitted { slot: Slot::Portrait, image: RgbaImage::from_pixel(600, 900, Rgba([1, 2, 3, 255])), source: String::new(), generated: false }
            .to_png()
            .unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        assert_eq!(dims(&fit_bytes(Slot::Portrait, &png).unwrap()), (600, 900));
        assert!(matches!(fit_bytes(Slot::Portrait, b"not an image"), Err(FitError::Decode(_))));
        assert!(matches!(fit_bytes(Slot::Portrait, b""), Err(FitError::Decode(_))));
    }

    #[test]
    fn oversized_sources_are_refused_by_the_decode_guard() {
        let strip = |w| {
            Fitted { slot: Slot::Logo, image: RgbaImage::from_pixel(w, 1, Rgba([1, 2, 3, 255])), source: String::new(), generated: false }
                .to_png()
                .unwrap()
        };
        assert!(matches!(decode(&strip(MAX_SOURCE_SIDE + 1)), Err(FitError::Decode(_))));
        assert!(decode(&strip(MAX_SOURCE_SIDE)).is_ok());
    }

    #[test]
    fn a_banner_is_not_squeezed_into_a_sliver_of_art_on_the_portrait_slot() {
        assert!(matches!(fit(Slot::Portrait, &solid(2000, 300, [1, 1, 1, 255])), Err(FitError::UnsuitableShape { slot: Slot::Portrait, .. })));
        assert!(matches!(fit(Slot::Portrait, &solid(1, 1000, [1, 1, 1, 255])), Err(FitError::UnsuitableShape { .. })));
        // 3:1 still gives a usable strip (600x200 on the slot).
        assert!(fit(Slot::Portrait, &solid(1200, 400, [1, 1, 1, 255])).is_ok());
    }

    #[test]
    fn a_tiny_logo_is_refused() {
        assert!(matches!(fit(Slot::Logo, &solid(16, 16, [255, 255, 255, 255])), Err(FitError::TooSmall { slot: Slot::Logo, .. })));
        assert!(fit(Slot::Logo, &solid(64, 20, [255, 255, 255, 255])).is_ok());
    }

    #[test]
    fn fits_slot_checks_the_size_each_slot_requires() {
        let f = |slot, w, h| Fitted { slot, image: RgbaImage::new(w, h), source: String::new(), generated: false }.fits_slot();
        assert!(f(Slot::Portrait, 600, 900) && !f(Slot::Portrait, 400, 200) && !f(Slot::Portrait, 900, 600));
        assert!(f(Slot::Wide, 920, 430) && !f(Slot::Wide, 920, 431));
        assert!(f(Slot::Hero, 1920, 620) && !f(Slot::Hero, 1920, 1080));
        assert!(f(Slot::Logo, 1280, 720) && f(Slot::Logo, 300, 100) && !f(Slot::Logo, 1281, 720) && !f(Slot::Logo, 10, 10) && !f(Slot::Logo, 0, 0));
        assert!(f(Slot::Icon, 512, 512) && f(Slot::Icon, 32, 32) && !f(Slot::Icon, 513, 513) && !f(Slot::Icon, 256, 128) && !f(Slot::Icon, 16, 16));
    }

    #[test]
    fn zero_sized_image_is_a_typed_error() {
        let empty = DynamicImage::ImageRgba8(RgbaImage::new(0, 10));
        assert!(matches!(fit(Slot::Logo, &empty), Err(FitError::Empty)));
    }
}
