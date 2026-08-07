//! Draws the Gaze application icon.
//!
//!     cargo run --release --example icon
//!
//! Writes a 1024x1024 PNG to `assets/icon.png`, which
//! `scripts/macos-release.sh` turns into the `.icns` inside `Gaze.app`. Run it
//! again whenever the face changes so the icon keeps matching the app.
//!
//! The PNG is written with stored deflate blocks, which keeps this free of
//! dependencies at the cost of a file around eighty times larger than it needs
//! to be. On macOS `sips` then re-compresses it in place.

use std::{env, fs, io, path::Path, process::Command};

const SIZE: u32 = 1024;
/// Samples per axis. The whole icon is edges, so this is what makes it smooth.
const SAMPLES: u32 = 4;

// macOS draws app icons as a rounded square inset from the canvas.
const PLATE_MIN: f32 = 100.0;
const PLATE_MAX: f32 = 924.0;
const PLATE_RADIUS: f32 = 185.0;

const PLATE_TOP: [f32; 3] = [0.376, 0.651, 0.718];
const PLATE_BOTTOM: [f32; 3] = [0.176, 0.376, 0.439];
const OUTLINE: [f32; 3] = [0.153, 0.149, 0.165];
const SCLERA: [f32; 3] = [1.0, 0.996, 0.980];
const IRIS: [f32; 3] = [0.286, 0.537, 0.604];
const PUPIL: [f32; 3] = [0.094, 0.122, 0.141];

const EYE_CENTER_Y: f32 = 512.0;
const EYE_OFFSET_X: f32 = 190.0;
const EYE_RADIUS_X: f32 = 170.0;
const EYE_RADIUS_Y: f32 = 195.0;
const EYE_OUTLINE: f32 = 17.0;
const IRIS_RADIUS: f32 = 78.0;
/// The irises look slightly down and to the right, the way the tray icon does.
const GAZE_X: f32 = 30.0;
const GAZE_Y: f32 = 26.0;

fn main() -> io::Result<()> {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/icon.png".into());
    let path = Path::new(&path);

    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            pixels.extend(resolve_pixel(x, y));
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, encode_png(SIZE, SIZE, &pixels))?;
    compress_in_place(path);
    println!(
        "wrote {} ({SIZE}x{SIZE}, {} KiB)",
        path.display(),
        fs::metadata(path)?.len() / 1024
    );
    Ok(())
}

/// Hands the file to `sips` to be written out again with real compression.
/// Absent or failing, the uncompressed PNG left behind is still a valid one.
fn compress_in_place(path: &Path) {
    let Ok(status) = Command::new("sips")
        .args(["-s", "format", "png"])
        .arg(path)
        .arg("--out")
        .arg(path)
        .stdout(std::process::Stdio::null())
        .status()
    else {
        eprintln!("note: sips is not available, leaving the PNG uncompressed");
        return;
    };
    if !status.success() {
        eprintln!("note: sips could not compress the PNG, leaving it as written");
    }
}

/// Averages the samples covering one pixel. The accumulation is premultiplied
/// so that the edge against the transparent corners does not darken.
fn resolve_pixel(x: u32, y: u32) -> [u8; 4] {
    let mut total = [0.0_f32; 4];
    for sample_y in 0..SAMPLES {
        for sample_x in 0..SAMPLES {
            let px = x as f32 + (sample_x as f32 + 0.5) / SAMPLES as f32;
            let py = y as f32 + (sample_y as f32 + 0.5) / SAMPLES as f32;
            let (color, alpha) = sample(px, py);
            for channel in 0..3 {
                total[channel] += color[channel] * alpha;
            }
            total[3] += alpha;
        }
    }

    let count = (SAMPLES * SAMPLES) as f32;
    let alpha = total[3] / count;
    if alpha <= 0.0 {
        return [0, 0, 0, 0];
    }

    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    [
        byte(total[0] / count / alpha),
        byte(total[1] / count / alpha),
        byte(total[2] / count / alpha),
        byte(alpha),
    ]
}

fn sample(x: f32, y: f32) -> ([f32; 3], f32) {
    for center_x in [512.0 - EYE_OFFSET_X, 512.0 + EYE_OFFSET_X] {
        if let Some(color) = eye_sample(x, y, center_x) {
            return (color, 1.0);
        }
    }
    if inside_plate(x, y) {
        return (plate_color(y), 1.0);
    }
    ([0.0; 3], 0.0)
}

fn eye_sample(x: f32, y: f32, center_x: f32) -> Option<[f32; 3]> {
    let outside = ellipse_reach(x, y, center_x, EYE_RADIUS_X, EYE_RADIUS_Y);
    if outside > 1.0 {
        return None;
    }

    let inner = ellipse_reach(
        x,
        y,
        center_x,
        EYE_RADIUS_X - EYE_OUTLINE,
        EYE_RADIUS_Y - EYE_OUTLINE,
    );
    if inner > 1.0 {
        return Some(OUTLINE);
    }

    let iris_x = center_x + GAZE_X;
    let iris_y = EYE_CENTER_Y + GAZE_Y;
    let to_iris = ((x - iris_x).powi(2) + (y - iris_y).powi(2)).sqrt();

    let highlight = ((x - (iris_x - IRIS_RADIUS * 0.27)).powi(2)
        + (y - (iris_y - IRIS_RADIUS * 0.27)).powi(2))
    .sqrt();
    if highlight <= IRIS_RADIUS * 0.16 {
        return Some(SCLERA);
    }
    if to_iris <= IRIS_RADIUS * 0.54 {
        return Some(PUPIL);
    }
    if to_iris <= IRIS_RADIUS {
        return Some(IRIS);
    }
    Some(SCLERA)
}

fn ellipse_reach(x: f32, y: f32, center_x: f32, radius_x: f32, radius_y: f32) -> f32 {
    ((x - center_x) / radius_x).powi(2) + ((y - EYE_CENTER_Y) / radius_y).powi(2)
}

fn inside_plate(x: f32, y: f32) -> bool {
    // Distance to the rounded square, measured from the nearest corner centre.
    let inset_min = PLATE_MIN + PLATE_RADIUS;
    let inset_max = PLATE_MAX - PLATE_RADIUS;
    let corner_x = (inset_min - x).max(x - inset_max).max(0.0);
    let corner_y = (inset_min - y).max(y - inset_max).max(0.0);

    let plate = PLATE_MIN..=PLATE_MAX;
    plate.contains(&x)
        && plate.contains(&y)
        && corner_x * corner_x + corner_y * corner_y <= PLATE_RADIUS * PLATE_RADIUS
}

fn plate_color(y: f32) -> [f32; 3] {
    let down = ((y - PLATE_MIN) / (PLATE_MAX - PLATE_MIN)).clamp(0.0, 1.0);
    let mut color = [0.0; 3];
    for channel in 0..3 {
        color[channel] = PLATE_TOP[channel] + (PLATE_BOTTOM[channel] - PLATE_TOP[channel]) * down;
    }
    color
}

// ---------------------------------------------------------------- PNG

fn encode_png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut header = Vec::new();
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]); // 8 bits per channel, truecolour with alpha

    let mut raw = Vec::with_capacity((height * (1 + width * 4)) as usize);
    for row in pixels.chunks_exact((width * 4) as usize) {
        raw.push(0); // no per-row filter
        raw.extend(row);
    }

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    write_chunk(&mut png, b"IHDR", &header);
    write_chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    write_chunk(&mut png, b"IEND", &[]);
    png
}

fn write_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend((data.len() as u32).to_be_bytes());
    png.extend(kind);
    png.extend(data);

    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend(kind);
    crc_input.extend(data);
    png.extend(crc32(&crc_input).to_be_bytes());
}

/// A zlib stream of uncompressed deflate blocks: valid everywhere, and short
/// enough to hold in one's head compared with a real compressor.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 65_535;

    let mut out = vec![0x78, 0x01];
    if data.is_empty() {
        out.extend([0x01, 0x00, 0x00, 0xff, 0xff]);
    }
    for (index, block) in data.chunks(BLOCK).enumerate() {
        let last = (index + 1) * BLOCK >= data.len();
        out.push(u8::from(last));
        let length = block.len() as u16;
        out.extend(length.to_le_bytes());
        out.extend((!length).to_le_bytes());
        out.extend(block);
    }
    out.extend(adler32(data).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let carry = crc & 1;
            crc >>= 1;
            if carry != 0 {
                crc ^= 0xedb8_8320;
            }
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut low, mut high) = (1_u32, 0_u32);
    for &byte in data {
        low = (low + u32::from(byte)) % 65_521;
        high = (high + low) % 65_521;
    }
    (high << 16) | low
}
