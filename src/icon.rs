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

/// Signed distance -> antialiased coverage over one pixel.
fn coverage(signed_distance: f32) -> f32 {
    (0.5 - signed_distance).clamp(0.0, 1.0)
}

fn segment_distance(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let t = (((px - x0) * dx + (py - y0) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((px - (x0 + t * dx)).powi(2) + (py - (y0 + t * dy)).powi(2)).sqrt()
}
