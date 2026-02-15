use anyhow::{Context, Result, bail};
use prost::Message;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

// ── Minimal protobuf definitions (matching TensorFlow event.proto / summary.proto) ──

/// A single TensorFlow Event record.
#[derive(Clone, PartialEq, Message)]
pub struct Event {
    /// Wall clock time of the event (seconds since epoch).
    #[prost(double, tag = "1")]
    pub wall_time: f64,

    /// Global step of the event.
    #[prost(int64, tag = "2")]
    pub step: i64,

    // oneof `what` — we only care about file_version and summary for scalar extraction.
    #[prost(string, optional, tag = "3")]
    pub file_version: Option<String>,

    #[prost(message, optional, tag = "5")]
    pub summary: Option<Summary>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Summary {
    #[prost(message, repeated, tag = "1")]
    pub value: Vec<SummaryValue>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SummaryValue {
    /// Tag name, e.g. "train/loss"
    #[prost(string, tag = "1")]
    pub tag: String,

    /// Simple scalar value.
    #[prost(float, optional, tag = "2")]
    pub simple_value: Option<f32>,
    // We skip other value types (image, histo, tensor, etc.) — only scalars matter.
}

// ── Public types ────────────────────────────────────────────────────────────

/// A parsed scalar event.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ScalarEvent {
    pub tag: String,
    pub step: i64,
    pub wall_time: f64,
    pub value: f64,
}

/// Parsed run payload used by the TUI.
#[derive(Debug, Clone)]
pub struct LoadedRun {
    pub scalars: BTreeMap<String, Vec<(f64, f64)>>,
    pub events: Vec<ScalarEvent>,
}

// ── Record-level reader ─────────────────────────────────────────────────────

/// TF record format per record:
///   uint64  length           (little-endian)
///   uint32  masked_crc32c(length_bytes)
///   byte    data[length]
///   uint32  masked_crc32c(data)

fn masked_crc32c(data: &[u8]) -> u32 {
    let crc = crc32c::crc32c(data);
    ((crc >> 15) | (crc << 17)).wrapping_add(0xa282_ead8)
}

fn read_exact_or_eof(cursor: &mut Cursor<&[u8]>, buf: &mut [u8]) -> Result<bool> {
    match cursor.read_exact(buf) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn read_record(cursor: &mut Cursor<&[u8]>) -> Result<Option<Vec<u8>>> {
    // Read 8-byte length
    let mut len_buf = [0u8; 8];
    if !read_exact_or_eof(cursor, &mut len_buf)? {
        return Ok(None);
    }
    let data_len = u64::from_le_bytes(len_buf) as usize;

    // Read 4-byte masked CRC of length
    let mut len_crc_buf = [0u8; 4];
    if !read_exact_or_eof(cursor, &mut len_crc_buf)? {
        return Ok(None);
    }
    let len_crc = u32::from_le_bytes(len_crc_buf);
    let expected_len_crc = masked_crc32c(&len_buf);
    if len_crc != expected_len_crc {
        bail!("CRC mismatch on record length");
    }

    // Read data
    let mut data = vec![0u8; data_len];
    if !read_exact_or_eof(cursor, &mut data)? {
        return Ok(None);
    }

    // Read 4-byte masked CRC of data
    let mut data_crc_buf = [0u8; 4];
    if !read_exact_or_eof(cursor, &mut data_crc_buf)? {
        return Ok(None);
    }
    let data_crc = u32::from_le_bytes(data_crc_buf);
    let expected_data_crc = masked_crc32c(&data);
    if data_crc != expected_data_crc {
        bail!("CRC mismatch on record data");
    }

    Ok(Some(data))
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Parse all scalar events from a single `.tfevents` file.
pub fn parse_events_file(path: &Path) -> Result<Vec<ScalarEvent>> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut cursor = Cursor::new(bytes.as_slice());
    let mut events = Vec::new();

    while let Some(data) = read_record(&mut cursor)? {
        let event = Event::decode(data.as_slice()).with_context(|| "decoding Event protobuf")?;

        if let Some(summary) = event.summary {
            for val in summary.value {
                if let Some(sv) = val.simple_value {
                    events.push(ScalarEvent {
                        tag: val.tag,
                        step: event.step,
                        wall_time: event.wall_time,
                        value: sv as f64,
                    });
                }
            }
        }
    }

    Ok(events)
}

fn discover_event_files(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    let mut files = Vec::new();
    for entry in walkdir(path)? {
        if entry
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .contains("tfevents")
        {
            files.push(entry);
        }
    }
    Ok(files)
}

fn load_events(path: &Path) -> Result<Vec<ScalarEvent>> {
    let mut all_events: Vec<ScalarEvent> = Vec::new();
    for entry in discover_event_files(path)? {
        match parse_events_file(&entry) {
            Ok(evts) => all_events.extend(evts),
            Err(e) => eprintln!("warning: skipping {}: {e}", entry.display()),
        }
    }
    Ok(all_events)
}

/// Discover `.tfevents` data under `path` and build both scalar series and raw events.
pub fn load_run(path: &Path) -> Result<LoadedRun> {
    let events = load_events(path)?;

    let mut scalars: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
    for ev in &events {
        scalars
            .entry(ev.tag.clone())
            .or_default()
            .push((ev.step as f64, ev.value));
    }

    // Sort each series by step
    for series in scalars.values_mut() {
        series.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    }

    Ok(LoadedRun { scalars, events })
}

/// Simple recursive directory walk (avoids adding walkdir dependency).
fn walkdir(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                files.extend(walkdir(&path)?);
            } else {
                files.push(path);
            }
        }
    }
    Ok(files)
}
