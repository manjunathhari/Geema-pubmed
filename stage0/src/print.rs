//! Print mode: one result as gzip then base64, in numbered lines.

use crate::job::valid_label;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use flate2::write::GzEncoder;
use flate2::{Compression, Crc};
use std::io::Write;
use std::path::Path;

/// Longest line printed, in characters.
pub const MAX_LINE: usize = 1_000;

/// Why a result can't be printed. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintError {
    BadLabel,
    NoResult,
    Failed,
}

/// Base64 characters on a line. With the prefix, a line stays under [`MAX_LINE`]:
/// an earlier result's name is at most 88 bytes (a 64-byte label, `.prev-`, a stamp).
const CHUNK: usize = 880;

/// Writes `bytes` to `w` as numbered lines and a final count and checksum.
pub fn write_encoded(label: &str, bytes: &[u8], w: &mut dyn Write) -> Result<(), PrintError> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::best());
    gz.write_all(bytes).map_err(|_| PrintError::Failed)?;
    let gz = gz.finish().map_err(|_| PrintError::Failed)?;
    let b64 = STANDARD.encode(&gz);
    let mut crc = Crc::new();
    crc.update(b64.as_bytes());
    let parts: Vec<&[u8]> = b64.as_bytes().chunks(CHUNK).collect();
    let total = parts.len();
    for (i, part) in parts.into_iter().enumerate() {
        let text = std::str::from_utf8(part).map_err(|_| PrintError::Failed)?;
        writeln!(w, "S0P {label} {}/{total} {text}", i + 1).map_err(|_| PrintError::Failed)?;
    }
    writeln!(
        w,
        "S0END {label} lines={total} b64_chars={} crc32={:08x} gz_bytes={} result_bytes={}",
        b64.len(),
        crc.sum(),
        gz.len(),
        bytes.len()
    )
    .map_err(|_| PrintError::Failed)
}

/// A label, or the name `run::save` gives an earlier result:
/// `<label>.prev-<8 digits>T<9 digits>`.
pub fn valid_result_name(name: &str) -> bool {
    match name.split_once(".prev-") {
        None => valid_label(name),
        Some((label, stamp)) => {
            let b = stamp.as_bytes();
            valid_label(label)
                && b.len() == 18
                && b.iter().enumerate().all(|(i, c)| {
                    if i == 8 {
                        *c == b'T'
                    } else {
                        c.is_ascii_digit()
                    }
                })
        }
    }
}

/// Prints the saved result for `label`, or an earlier one by its full name.
pub fn print_result(out: &Path, label: &str, w: &mut dyn Write) -> Result<(), PrintError> {
    if !valid_result_name(label) {
        return Err(PrintError::BadLabel);
    }
    let bytes =
        std::fs::read(out.join(format!("{label}.json"))).map_err(|_| PrintError::NoResult)?;
    write_encoded(label, &bytes, w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::io::Read;

    /// What a reader of the logs does, in any order of lines.
    fn decode(text: &str) -> (Vec<u8>, usize) {
        let mut parts: Vec<(usize, String)> = Vec::new();
        let (mut total, mut crc_said, mut chars) = (0, String::new(), 0);
        let mut body_lines = 0;
        for l in text.lines() {
            assert!(l.chars().count() <= MAX_LINE, "line too long");
            let f: Vec<&str> = l.split(' ').collect();
            match f[0] {
                "S0P" => {
                    let (n, t) = f[2].split_once('/').unwrap();
                    total = t.parse().unwrap();
                    parts.push((n.parse().unwrap(), f[3].to_string()));
                    body_lines += 1;
                }
                "S0END" => {
                    for kv in &f[2..] {
                        let (k, v) = kv.split_once('=').unwrap();
                        match k {
                            "lines" => assert_eq!(v.parse::<usize>().unwrap(), body_lines),
                            "b64_chars" => chars = v.parse().unwrap(),
                            "crc32" => crc_said = v.to_string(),
                            _ => {}
                        }
                    }
                }
                other => panic!("unexpected line kind {other}"),
            }
        }
        assert_eq!(parts.len(), total);
        parts.sort();
        let b64: String = parts.into_iter().map(|(_, p)| p).collect();
        assert_eq!(b64.len(), chars);
        let mut crc = Crc::new();
        crc.update(b64.as_bytes());
        assert_eq!(crc_said, format!("{:08x}", crc.sum()));
        let gz = STANDARD.decode(b64).unwrap();
        let mut out = Vec::new();
        GzDecoder::new(&gz[..]).read_to_end(&mut out).unwrap();
        (out, body_lines)
    }

    /// Bytes that do not compress: the worst case for line count.
    fn noise(n: usize) -> Vec<u8> {
        let mut x: u64 = 0x1234_5678_9abc_def1;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    }

    #[test]
    fn it_round_trips_and_a_40_kb_answer_fits_well_under_300_lines() {
        let bytes = noise(40 * 1024);
        let mut out = Vec::new();
        write_encoded("a-1", &bytes, &mut out).unwrap();
        let (back, lines) = decode(std::str::from_utf8(&out).unwrap());
        assert_eq!(back, bytes);
        assert!(lines < 100, "{lines} lines");
    }

    #[test]
    fn a_small_result_is_one_line_and_a_final_line() {
        let mut out = Vec::new();
        write_encoded("a-1", b"made up", &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.starts_with("S0P a-1 1/1 "));
        assert!(
            text.lines()
                .last()
                .unwrap()
                .starts_with("S0END a-1 lines=1 ")
        );
        assert_eq!(decode(&text).0, b"made up");
    }

    #[test]
    fn a_long_label_still_fits_the_line_limit() {
        let label = "x".repeat(64);
        let mut out = Vec::new();
        write_encoded(&label, &noise(20_000), &mut out).unwrap();
        decode(std::str::from_utf8(&out).unwrap());
    }

    #[test]
    fn the_longest_earlier_result_name_still_fits_the_line_limit() {
        let name = format!("{}.prev-20261106T000000000", "x".repeat(64));
        assert!(valid_result_name(&name));
        let mut out = Vec::new();
        // Enough lines that the line numbers reach 3 digits.
        write_encoded(&name, &noise(120_000), &mut out).unwrap();
        decode(std::str::from_utf8(&out).unwrap());
    }

    #[test]
    fn only_a_label_or_a_name_save_makes_is_accepted() {
        for good in ["a", "a-1", "a.prev-20261106T000000000"] {
            assert!(valid_result_name(good), "{good}");
        }
        for bad in [
            "",
            "a.b",
            ".prev-20261106T000000000",
            "a.prev-",
            "a.prev-2026",
            "a.prev-20261106X000000000",
            "a.prev-20261106T00000000/",
            "a.prev-20261106T000000000.prev-20261106T000000000",
            "../a.prev-20261106T000000000",
        ] {
            assert!(!valid_result_name(bad), "{bad}");
        }
    }

    #[test]
    fn labels_are_checked_and_a_missing_result_is_reported() {
        let dir = std::env::temp_dir().join(format!("stage0-print-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.json"), b"made up").unwrap();
        let mut out = Vec::new();
        assert_eq!(
            print_result(&dir, "../a", &mut out),
            Err(PrintError::BadLabel)
        );
        assert_eq!(print_result(&dir, "b", &mut out), Err(PrintError::NoResult));
        assert!(out.is_empty());
        print_result(&dir, "a", &mut out).unwrap();
        assert_eq!(decode(std::str::from_utf8(&out).unwrap()).0, b"made up");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
