//! A small, self-made ICC v2.1 display profile for sRGB (matrix/TRC type).
//!
//! PDF/A needs an embedded output-intent profile. Rather than redistribute somebody else's
//! profile file (and its licence), the profile is generated from the published sRGB
//! primaries (D50-adapted, as used by the ICC's own sRGB profiles) and the sRGB transfer
//! curve sampled at 1024 points. The result is a few kilobytes.

const PROFILE_DESCRIPTION: &str = "sRGB IEC61966-2.1 (BergPDF generated)";
const COPYRIGHT: &str = "No copyright, generated from the public sRGB specification";

/// s15Fixed16 encoding.
fn s15(v: f64) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

fn srgb_eotf(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn pad4(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
}

fn xyz_tag(x: f64, y: f64, z: f64) -> Vec<u8> {
    let mut t = b"XYZ \0\0\0\0".to_vec();
    for v in [x, y, z] {
        t.extend_from_slice(&s15(v));
    }
    t
}

fn text_tag(s: &str) -> Vec<u8> {
    let mut t = b"text\0\0\0\0".to_vec();
    t.extend_from_slice(s.as_bytes());
    t.push(0);
    t
}

fn desc_tag(s: &str) -> Vec<u8> {
    // textDescriptionType: ASCII, empty Unicode and ScriptCode parts.
    let mut t = b"desc\0\0\0\0".to_vec();
    t.extend_from_slice(&(s.len() as u32 + 1).to_be_bytes());
    t.extend_from_slice(s.as_bytes());
    t.push(0);
    t.extend_from_slice(&0u32.to_be_bytes()); // Unicode language code
    t.extend_from_slice(&0u32.to_be_bytes()); // Unicode count
    t.extend_from_slice(&0u16.to_be_bytes()); // ScriptCode code
    t.push(0); // ScriptCode count
    t.extend_from_slice(&[0u8; 67]);
    t
}

fn curve_tag() -> Vec<u8> {
    const N: usize = 1024;
    let mut t = b"curv\0\0\0\0".to_vec();
    t.extend_from_slice(&(N as u32).to_be_bytes());
    for i in 0..N {
        let v = srgb_eotf(i as f64 / (N - 1) as f64);
        t.extend_from_slice(&((v * 65535.0).round() as u16).to_be_bytes());
    }
    t
}

/// Assemble a profile from tags (signature, data); identical data may be shared by passing the
/// same signature list entry twice via `shared`.
fn assemble(
    color_space: &[u8; 4],
    tags: &[([u8; 4], Vec<u8>)],
    shared: &[([u8; 4], usize)],
) -> Vec<u8> {
    let count = tags.len() + shared.len();
    let table_len = 4 + 12 * count;
    let mut offset = 128 + table_len;
    let mut table = Vec::new();
    let mut body = Vec::new();
    table.extend_from_slice(&(count as u32).to_be_bytes());
    let mut offsets = Vec::new();
    for (sig, data) in tags {
        let mut d = data.clone();
        let size = d.len();
        pad4(&mut d);
        table.extend_from_slice(sig);
        table.extend_from_slice(&(offset as u32).to_be_bytes());
        table.extend_from_slice(&(size as u32).to_be_bytes());
        offsets.push((offset, size));
        offset += d.len();
        body.extend_from_slice(&d);
    }
    for (sig, idx) in shared {
        let (off, size) = offsets[*idx];
        table.extend_from_slice(sig);
        table.extend_from_slice(&(off as u32).to_be_bytes());
        table.extend_from_slice(&(size as u32).to_be_bytes());
    }
    let total = 128 + table.len() + body.len();
    let mut h = Vec::with_capacity(128);
    h.extend_from_slice(&(total as u32).to_be_bytes());
    h.extend_from_slice(&[0; 4]); // preferred CMM
    h.extend_from_slice(&[2, 0x10, 0, 0]); // version 2.1.0
    h.extend_from_slice(b"mntr");
    h.extend_from_slice(color_space);
    h.extend_from_slice(b"XYZ ");
    for v in [2026u16, 1, 1, 0, 0, 0] {
        h.extend_from_slice(&v.to_be_bytes());
    }
    h.extend_from_slice(b"acsp");
    h.extend_from_slice(&[0; 4]); // platform
    h.extend_from_slice(&[0; 4]); // flags
    h.extend_from_slice(&[0; 4]); // manufacturer
    h.extend_from_slice(&[0; 4]); // model
    h.extend_from_slice(&[0; 8]); // attributes
    h.extend_from_slice(&[0; 4]); // rendering intent: perceptual
    for v in [0.9642, 1.0, 0.8249] {
        h.extend_from_slice(&s15(v)); // PCS illuminant D50
    }
    h.extend_from_slice(&[0; 4]); // creator
    h.extend_from_slice(&[0; 16]); // profile ID
    h.extend_from_slice(&[0; 28]); // reserved
    debug_assert_eq!(h.len(), 128);
    let mut out = h;
    out.extend_from_slice(&table);
    out.extend_from_slice(&body);
    out
}

/// The generated sRGB profile (RGB, v2.1).
pub fn srgb_profile() -> Vec<u8> {
    let tags = vec![
        (*b"desc", desc_tag(PROFILE_DESCRIPTION)),
        (*b"cprt", text_tag(COPYRIGHT)),
        (*b"wtpt", xyz_tag(0.9642, 1.0, 0.8249)),
        (*b"rXYZ", xyz_tag(0.436_065_7, 0.222_493_2, 0.013_923_9)),
        (*b"gXYZ", xyz_tag(0.385_151_2, 0.716_886_0, 0.097_081_2)),
        (*b"bXYZ", xyz_tag(0.143_078_4, 0.060_621_0, 0.714_173_3)),
        (*b"rTRC", curve_tag()),
    ];
    // The green and blue curves reuse the red curve's data.
    assemble(b"RGB ", &tags, &[(*b"gTRC", 6), (*b"bTRC", 6)])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn u32_at(b: &[u8], i: usize) -> u32 {
        u32::from_be_bytes(b[i..i + 4].try_into().unwrap())
    }

    #[test]
    fn header_and_tag_table_are_consistent() {
        let p = srgb_profile();
        assert_eq!(
            u32_at(&p, 0) as usize,
            p.len(),
            "declared size equals real size"
        );
        assert_eq!(&p[36..40], b"acsp");
        assert_eq!(&p[12..16], b"mntr");
        assert_eq!(&p[16..20], b"RGB ");
        assert_eq!(p[8], 2);
        let n = u32_at(&p, 128) as usize;
        assert_eq!(n, 9);
        for i in 0..n {
            let e = 132 + i * 12;
            let (off, size) = (u32_at(&p, e + 4) as usize, u32_at(&p, e + 8) as usize);
            assert_eq!(off % 4, 0, "tags are 4-byte aligned");
            assert!(off + size <= p.len());
        }
    }

    #[test]
    fn transfer_curve_endpoints_and_midpoint() {
        let t = curve_tag();
        let n = u32_at(&t, 8) as usize;
        assert_eq!(n, 1024);
        let at = |i: usize| u16::from_be_bytes([t[12 + 2 * i], t[13 + 2 * i]]);
        assert_eq!(at(0), 0);
        assert_eq!(at(1023), 65535);
        // sRGB 0.5 encodes ≈ 0.214 linear.
        let mid = f64::from(at(512)) / 65535.0;
        assert!((mid - 0.2140).abs() < 0.003, "{mid}");
    }

    #[test]
    fn white_point_and_primaries_sum_to_d50() {
        let sum = |a: f64, b: f64, c: f64| a + b + c;
        assert!((sum(0.4360657, 0.3851512, 0.1430784) - 0.9642).abs() < 1e-4);
        assert!((sum(0.2224932, 0.7168860, 0.0606210) - 1.0).abs() < 1e-4);
        assert!((sum(0.0139239, 0.0970812, 0.7141733) - 0.8252).abs() < 1e-3);
    }
}
