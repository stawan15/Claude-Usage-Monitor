use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};

/// Tails append-only JSONL files, handing each new complete line to a callback.
#[derive(Default)]
pub struct JsonlTailer {
    offsets: HashMap<PathBuf, u64>,
}

impl JsonlTailer {
    pub fn scan(
        &mut self,
        roots: &[PathBuf],
        cutoff: DateTime<Utc>,
        include: impl Fn(&Path) -> bool,
        mut on_line: impl FnMut(&Path, &[u8]),
    ) {
        let cutoff: SystemTime = cutoff.into();
        let mut files = Vec::new();
        for root in roots {
            walk(root, &mut files);
        }

        for path in files.into_iter().filter(|p| include(p)) {
            let Ok(meta) = fs::metadata(&path) else { continue };
            let size = meta.len();
            let known = self.offsets.get(&path).copied();
            let mut offset = known.unwrap_or(0);
            if size < offset {
                offset = 0; // truncated or replaced
            }
            if size == offset {
                continue;
            }

            // First sighting of a stale file: skip its history but tail it from here on.
            if known.is_none() && meta.modified().is_ok_and(|m| m < cutoff) {
                self.offsets.insert(path, size);
                continue;
            }

            let consumed = read_lines(&path, offset, size - offset, &mut on_line);
            self.offsets.insert(path, offset + consumed);
        }
    }
}

/// Recursively lists regular files, skipping hidden entries.
pub fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, files),
            Ok(t) if t.is_file() => files.push(path),
            _ => {}
        }
    }
}

/// Returns bytes consumed. A trailing partial line is left for the next scan.
fn read_lines(path: &Path, offset: u64, len: u64, on_line: &mut impl FnMut(&Path, &[u8])) -> u64 {
    let Ok(mut file) = File::open(path) else { return 0 };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return 0;
    }
    let mut data = Vec::with_capacity(len as usize);
    if file.take(len).read_to_end(&mut data).is_err() {
        return 0;
    }
    let Some(last_newline) = data.iter().rposition(|&b| b == b'\n') else { return 0 };

    let complete = &data[..=last_newline];
    for line in complete.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if !line.is_empty() {
            on_line(path, line);
        }
    }
    complete.len() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_only_new_complete_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        let mut file = File::create(&path).unwrap();
        write!(file, "one\ntwo\npart").unwrap();

        let mut tailer = JsonlTailer::default();
        let roots = [dir.path().to_path_buf()];
        let cutoff = Utc::now() - chrono::Duration::days(1);

        let mut lines = Vec::new();
        tailer.scan(&roots, cutoff, |_| true, |_, l| lines.push(String::from_utf8_lossy(l).into_owned()));
        assert_eq!(lines, ["one", "two"]);

        write!(file, "ial\nthree\n").unwrap();
        lines.clear();
        tailer.scan(&roots, cutoff, |_| true, |_, l| lines.push(String::from_utf8_lossy(l).into_owned()));
        assert_eq!(lines, ["partial", "three"]);
    }
}
