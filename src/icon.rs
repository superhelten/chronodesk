//! Procedurally drawn clock-face icon, so the app ships without asset files.

pub struct Rgba {
    pub rgba: Vec<u8>,
    pub size: u32,
}

/// White clock face when unlocked; amber when click-through is locked.
pub fn app_icon(size: u32, locked: bool) -> Rgba {
    let color: [u8; 3] = if locked { [245, 165, 36] } else { [240, 240, 240] };
    let s = size as f32;
    let c = s / 2.0;
    let radius = s * 0.40;
    let ring = s * 0.085;
    let hand = s * 0.06;

    // Hands: 12 o'clock and 3 o'clock-ish (10:10 reads as "clock" at tiny sizes).
    let hands = [(c, c, c, c - radius * 0.62), (c, c, c + radius * 0.48, c + radius * 0.10)];

    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let dist = ((px - c).powi(2) + (py - c).powi(2)).sqrt();
            let ring_cov = coverage((dist - radius).abs() - ring / 2.0);
            let hand_cov = hands
                .iter()
                .map(|&(x0, y0, x1, y1)| coverage(segment_distance(px, py, x0, y0, x1, y1) - hand / 2.0))
                .fold(0.0, f32::max);
            let a = ring_cov.max(hand_cov);
            if a > 0.0 {
                let i = ((y * size + x) * 4) as usize;
                rgba[i..i + 3].copy_from_slice(&color);
                rgba[i + 3] = (a * 255.0).round() as u8;
            }
        }
    }
    Rgba { rgba, size }
}

/// The same face as an `.ico` file, one image per size, for the places that
/// want a file rather than pixels: the Start menu shortcut and the entry under
/// Installed apps. Classic 32-bit DIB entries, which everything reads.
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&size| dib(&app_icon(size, false))).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len();
    for (&size, image) in sizes.iter().zip(&images) {
        // A byte each; 0 stands for 256.
        out.extend_from_slice(&[size as u8, size as u8, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(image.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += image.len();
    }
    images.iter().for_each(|image| out.extend_from_slice(image));
    out
}

/// One icon image: a `BITMAPINFOHEADER` whose height counts the colour rows
/// and the mask rows, the pixels bottom-up as BGRA, then a 1-bit mask (all
/// zero, as the alpha channel already says what is transparent) in rows padded
/// to 32 bits.
fn dib(icon: &Rgba) -> Vec<u8> {
    let size = icon.size;
    let mask_row = size.div_ceil(32) as usize * 4;
    let mut out = Vec::with_capacity(40 + (size * size * 4) as usize + mask_row * size as usize);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&(size as i32 * 2).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for row in icon.rgba.chunks_exact(size as usize * 4).rev() {
        for px in row.as_chunks::<4>().0 {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    out.resize(out.len() + mask_row * size as usize, 0);
    out
}

/// Signed distance -> antialiased coverage over one pixel.
fn coverage(signed_distance: f32) -> f32 {
    (0.5 - signed_distance).clamp(0.0, 1.0)
}

fn segment_distance(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let t = (((px - x0) * dx + (py - y0) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((px - (x0 + t * dx)).powi(2) + (py - (y0 + t * dy)).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ico_directory_describes_the_images_that_follow() {
        let sizes = [16, 32, 48];
        let file = ico(&sizes);
        assert_eq!(&file[..6], &[0, 0, 1, 0, 3, 0], "reserved, type icon, three images");
        let mut expected_offset = 6 + 16 * sizes.len();
        for (i, size) in sizes.into_iter().enumerate() {
            let entry = &file[6 + 16 * i..6 + 16 * (i + 1)];
            assert_eq!((entry[0], entry[1]), (size as u8, size as u8));
            let len = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
            let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
            assert_eq!(offset, expected_offset);
            assert_eq!(len, 40 + (size * size * 4) as usize + size.div_ceil(32) as usize * 4 * size as usize);
            // The header: 40 bytes, the width, and twice the height.
            let header = &file[offset..offset + 12];
            assert_eq!(u32::from_le_bytes(header[..4].try_into().unwrap()), 40);
            assert_eq!(i32::from_le_bytes(header[4..8].try_into().unwrap()), size as i32);
            assert_eq!(i32::from_le_bytes(header[8..12].try_into().unwrap()), size as i32 * 2);
            expected_offset += len;
        }
        assert_eq!(file.len(), expected_offset);
    }

    #[test]
    fn pixels_are_stored_bottom_up_as_bgra() {
        let icon = Rgba { rgba: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16], size: 2 };
        let image = dib(&icon);
        assert_eq!(&image[40..56], &[11, 10, 9, 12, 15, 14, 13, 16, 3, 2, 1, 4, 7, 6, 5, 8]);
        assert_eq!(image.len(), 40 + 16 + 8, "and a two-row mask, four bytes a row");
    }
}
