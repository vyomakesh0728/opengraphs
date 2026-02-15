use anyhow::{Context, Result, bail};
use prost::Message;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;

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

/// A parsed scalar event.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ScalarEvent {
    pub tag: String,
    pub step: i64,
    pub wall_time: f64,
    pub value: f64,
    pub run_name: String,
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

fn read_record(cursor: &mut Cursor<&[u8]>) -> Result<Option<Vec<u8>>> {
    // Read 8-byte length
    let mut len_buf = [0u8; 8];
    match cursor.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let data_len = u64::from_le_bytes(len_buf) as usize;

    // Read 4-byte masked CRC of length
    let mut len_crc_buf = [0u8; 4];
    cursor.read_exact(&mut len_crc_buf)?;
    let len_crc = u32::from_le_bytes(len_crc_buf);
    let expected_len_crc = masked_crc32c(&len_buf);
    if len_crc != expected_len_crc {
        bail!("CRC mismatch on record length");
    }

    // Read data
    let mut data = vec![0u8; data_len];
    cursor.read_exact(&mut data)?;

    // Read 4-byte masked CRC of data
    let mut data_crc_buf = [0u8; 4];
    cursor.read_exact(&mut data_crc_buf)?;
    let data_crc = u32::from_le_bytes(data_crc_buf);
    let expected_data_crc = masked_crc32c(&data);
    if data_crc != expected_data_crc {
        bail!("CRC mismatch on record data");
    }

    Ok(Some(data))
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Parse all scalar events from a single `.tfevents` file.
#[allow(dead_code)]
pub fn parse_events_file(path: &Path, run_name: &str) -> Result<Vec<ScalarEvent>> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut cursor = Cursor::new(bytes.as_slice());
    let mut events = Vec::new();

    while let Some(data) = read_record(&mut cursor)? {
        let event = Event::decode(data.as_slice())
            .with_context(|| "decoding Event protobuf")?;

        if let Some(summary) = event.summary {
            for val in summary.value {
                if let Some(sv) = val.simple_value {
                    events.push(ScalarEvent {
                        tag: val.tag,
                        step: event.step,
                        wall_time: event.wall_time,
                        value: sv as f64,
                        run_name: run_name.to_string(),
                    });
                }
            }
        }
    }

    Ok(events)
}

/// Discover all `.tfevents` files in a directory (recursively) and parse them.
/// Returns tag → { run_name → sorted [(step, value)] }.
/// Each first-level subdirectory is treated as a separate "run".
#[allow(dead_code)]
pub fn load_scalars(dir: &Path) -> Result<BTreeMap<String, BTreeMap<String, Vec<(f64, f64)>>>> {
    let reader = IncrementalReader::new(dir)?;
    let scalars = reader.scalars.clone();
    // Drop reader — use IncrementalReader::new() directly if you want live updates
    let _ = reader;
    Ok(scalars)
}

// ── Incremental Reader ──────────────────────────────────────────────────────

use std::collections::HashMap;
use std::path::PathBuf;

/// Tracks byte offsets into tfevents files to enable incremental parsing.
/// Only reads newly appended bytes on each `poll()` call.
pub struct IncrementalReader {
    /// Root directory being watched
    root: PathBuf,
    /// Whether root is a single file
    is_file: bool,
    /// file_path → (run_name, last_read_byte_offset)
    file_offsets: HashMap<PathBuf, (String, u64)>,
    /// Accumulated scalars: tag → { run → sorted [(step, value)] }
    pub scalars: BTreeMap<String, BTreeMap<String, Vec<(f64, f64)>>>,
    /// Total events parsed so far
    pub total_events: usize,
    /// Max step seen
    pub max_step: i64,
    /// All log lines
    pub log_lines: Vec<String>,
    /// Ordered run names
    pub run_names: Vec<String>,
}

impl IncrementalReader {
    /// Create a new reader, performing the initial full parse.
    pub fn new(dir: &Path) -> Result<Self> {
        let is_file = dir.is_file();
        let mut reader = Self {
            root: dir.to_path_buf(),
            is_file,
            file_offsets: HashMap::new(),
            scalars: BTreeMap::new(),
            total_events: 0,
            max_step: 0,
            log_lines: vec!["-- parsed events log --".to_string(), String::new()],
            run_names: Vec::new(),
        };

        // Discover and parse all files initially
        let files = reader.discover_files()?;
        for (path, run_name) in files {
            reader.parse_file_from_offset(&path, &run_name, 0)?;
        }

        reader.rebuild_run_names();
        Ok(reader)
    }

    /// Poll for new data. Reads only bytes appended since last read.
    /// Also discovers any new files that appeared.
    /// Returns true if any new data was found.
    pub fn poll(&mut self) -> Result<bool> {
        let mut found_new = false;

        // Check for new files
        let current_files = self.discover_files()?;
        for (path, run_name) in &current_files {
            if !self.file_offsets.contains_key(path) {
                // New file discovered
                self.parse_file_from_offset(path, run_name, 0)?;
                found_new = true;
            }
        }

        // Re-read existing files from last offset
        let existing: Vec<(PathBuf, String, u64)> = self.file_offsets.iter()
            .map(|(p, (r, o))| (p.clone(), r.clone(), *o))
            .collect();

        for (path, run_name, offset) in existing {
            // Check if file has grown
            if let Ok(meta) = std::fs::metadata(&path) {
                let file_len = meta.len();
                if file_len > offset {
                    self.parse_file_from_offset(&path, &run_name, offset)?;
                    found_new = true;
                }
            }
        }

        if found_new {
            // Re-sort series that may have new data
            for runs in self.scalars.values_mut() {
                for series in runs.values_mut() {
                    series.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                }
            }
            self.rebuild_run_names();
        }

        Ok(found_new)
    }

    /// Parse a file starting at `offset` bytes. Updates internal state.
    fn parse_file_from_offset(&mut self, path: &Path, run_name: &str, offset: u64) -> Result<()> {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        if (offset as usize) >= bytes.len() {
            // No new data
            self.file_offsets.insert(path.to_path_buf(), (run_name.to_string(), bytes.len() as u64));
            return Ok(());
        }

        let slice = &bytes[offset as usize..];
        let mut cursor = Cursor::new(slice);

        loop {
            let pos_before = cursor.position();
            match read_record(&mut cursor) {
                Ok(Some(data)) => {
                    match Event::decode(data.as_slice()) {
                        Ok(event) => {
                            if event.step > self.max_step {
                                self.max_step = event.step;
                            }
                            if let Some(summary) = event.summary {
                                for val in summary.value {
                                    if let Some(sv) = val.simple_value {
                                        self.total_events += 1;
                                        let step = event.step as f64;
                                        let value = sv as f64;

                                        self.scalars
                                            .entry(val.tag.clone())
                                            .or_default()
                                            .entry(run_name.to_string())
                                            .or_default()
                                            .push((step, value));

                                        self.log_lines.push(format!(
                                            "step {:>6} │ {:<30} │ {:.6} │ {}",
                                            event.step, val.tag, value, run_name
                                        ));
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            // Skip malformed record
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    // Partial record (file still being written) — rewind to before this record
                    let final_offset = offset + pos_before;
                    self.file_offsets.insert(path.to_path_buf(), (run_name.to_string(), final_offset));
                    return Ok(());
                }
            }
        }

        // Successfully read to end
        self.file_offsets.insert(path.to_path_buf(), (run_name.to_string(), bytes.len() as u64));
        Ok(())
    }

    /// Discover all tfevents files under root, returning (path, run_name) pairs.
    fn discover_files(&self) -> Result<Vec<(PathBuf, String)>> {
        let mut result = Vec::new();

        if self.is_file {
            let run_name = self.root.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            result.push((self.root.clone(), run_name));
            return Ok(result);
        }

        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let run_name = path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                for file_path in walkdir(&path)? {
                    if file_path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .contains("tfevents")
                    {
                        result.push((file_path, run_name.clone()));
                    }
                }
            } else if path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .contains("tfevents")
            {
                result.push((path, "default".to_string()));
            }
        }

        Ok(result)
    }

    /// Rebuild the ordered list of run names from current scalars.
    fn rebuild_run_names(&mut self) {
        let mut run_set = std::collections::BTreeSet::new();
        for runs in self.scalars.values() {
            for run_name in runs.keys() {
                run_set.insert(run_name.clone());
            }
        }
        self.run_names = run_set.into_iter().collect();
    }
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
