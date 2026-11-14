//! Bounded, deterministic reads shared by the session adapters.
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Water-fill the byte budget: small files release their unused share to large
/// files before any reads, so recent large files cannot starve later sessions.
pub(super) fn fair_allocations(files: &[PathBuf], budget: u64, per_file: u64) -> Vec<u64> {
    let sizes: Vec<_> = files
        .iter()
        .map(|path| fs::metadata(path).map_or(0, |m| m.len().min(per_file)))
        .collect();
    let mut allocations = vec![0; files.len()];
    let mut remaining = budget;
    loop {
        let pending: Vec<_> = sizes
            .iter()
            .enumerate()
            .filter(|(i, size)| allocations[*i] < **size)
            .map(|(i, _)| i)
            .collect();
        if pending.is_empty() || remaining == 0 {
            break;
        }
        let share = (remaining / pending.len() as u64).max(1);
        for index in pending {
            let assigned = share.min(sizes[index] - allocations[index]).min(remaining);
            allocations[index] += assigned;
            remaining -= assigned;
        }
    }
    allocations
}

/// Disjoint beginning/middle/end regions, each aligned inward to complete JSONL
/// records. Never stitch JSON fragments across unread gaps. A giant record is
/// discarded without extending the read budget or manufacturing a partial event.
pub(super) fn read_windows(
    path: &Path,
    file_bytes: u64,
    maximum: u64,
    jsonl: bool,
) -> io::Result<(Vec<Vec<u8>>, u64, bool)> {
    let mut file = File::open(path)?;
    let maximum = maximum.min(file_bytes);
    let ranges = if maximum == file_bytes {
        vec![(0, maximum)]
    } else if maximum < 3 {
        vec![(file_bytes - maximum, maximum)]
    } else {
        let width = maximum / 3;
        vec![
            (0, width),
            ((file_bytes - width) / 2, width),
            (file_bytes - (maximum - 2 * width), maximum - 2 * width),
        ]
    };
    let mut chunks = Vec::new();
    let mut bytes_read = 0;
    let mut alignment_lost = false;
    for (offset, length) in ranges {
        file.seek(SeekFrom::Start(offset))?;
        let mut chunk = Vec::with_capacity(length as usize);
        (&mut file).take(length).read_to_end(&mut chunk)?;
        bytes_read += chunk.len() as u64;
        if jsonl {
            if offset > 0 {
                if let Some(newline) = chunk.iter().position(|byte| *byte == b'\n') {
                    alignment_lost |= newline > 0;
                    chunk.drain(..=newline);
                } else {
                    alignment_lost |= !chunk.is_empty();
                    chunk.clear();
                }
            }
            if offset + length < file_bytes && !chunk.ends_with(b"\n") {
                alignment_lost |= !chunk.is_empty();
                if let Some(newline) = chunk.iter().rposition(|byte| *byte == b'\n') {
                    chunk.truncate(newline + 1);
                } else {
                    chunk.clear();
                }
            }
        }
        chunks.push(chunk);
    }
    Ok((chunks, bytes_read, alignment_lost))
}

/// Cheap candidate selection never creates Evidence. Escaped Unicode is kept
/// because a JSON escape can hide an otherwise exact Skill reference.
pub(super) fn has_usage_candidate(line: &[u8], _names: Option<&aho_corasick::AhoCorasick>) -> bool {
    static MARKERS: std::sync::OnceLock<aho_corasick::AhoCorasick> = std::sync::OnceLock::new();
    static EVENTS: std::sync::OnceLock<aho_corasick::AhoCorasick> = std::sync::OnceLock::new();
    let markers = MARKERS.get_or_init(|| {
        aho_corasick::AhoCorasickBuilder::new()
            .ascii_case_insensitive(true)
            .build(["skill", "\\u"])
            .expect("fixed patterns")
    });
    let events = EVENTS.get_or_init(|| {
        aho_corasick::AhoCorasickBuilder::new()
            .ascii_case_insensitive(true)
            .build([
                "toolcall",
                "tool_use",
                "function_call",
                "custom_tool_call",
                "loaded_skill",
                "applied_skill",
                "invoked_skill",
                "outcome_skill",
                "selected_skill",
                "matched_skill",
                "load_skill",
                "invoke_skill",
                "apply_skill",
                "skill_outcome",
                "match_skill",
                "\"read\"",
                "\"read_file\"",
                "\"load\"",
                "\"open\"",
            ])
            .expect("fixed patterns")
    });
    markers.is_match(line) && events.is_match(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn allocation_releases_small_files_and_reaches_many_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let files: Vec<_> = (0..80)
            .map(|index| {
                let path = directory.path().join(format!("{index}.jsonl"));
                File::create(&path)
                    .unwrap()
                    .set_len(if index == 0 { 1 } else { 10_000 })
                    .unwrap();
                path
            })
            .collect();
        let allocations = fair_allocations(&files, 80_000, 10_000);
        assert_eq!(allocations.iter().sum::<u64>(), 80_000);
        assert_eq!(allocations[0], 1);
        assert!(allocations[1..].iter().all(|value| *value >= 1_012));
        assert_eq!(allocations, fair_allocations(&files, 80_000, 10_000));
    }

    #[test]
    fn disjoint_windows_keep_head_middle_tail_without_duplicate_records() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        let mut file = File::create(&path).unwrap();
        for index in 100..400 {
            writeln!(file, "{{\"event\":{index:03}}}").unwrap();
        }
        let size = file.metadata().unwrap().len();
        let (chunks, bytes, _) = read_windows(&path, size, 900, true).unwrap();
        assert_eq!(bytes, 900);
        let records: Vec<serde_json::Value> = chunks
            .iter()
            .flat_map(|chunk| chunk.split_inclusive(|byte| *byte == b'\n'))
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(records.first().unwrap()["event"], 100);
        assert!(records.iter().any(|record| record["event"] == 250));
        assert_eq!(records.last().unwrap()["event"], 399);
        let ids: std::collections::BTreeSet<_> = records
            .iter()
            .map(|record| record["event"].as_u64().unwrap())
            .collect();
        assert_eq!(ids.len(), records.len());
    }

    #[test]
    fn giant_lines_do_not_extend_reads_or_join_fragments_into_events() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        let mut file = File::create(&path).unwrap();
        file.write_all(&vec![b'x'; 100_000]).unwrap();
        let (chunks, bytes, alignment_lost) = read_windows(&path, 100_000, 900, true).unwrap();
        assert_eq!(bytes, 900);
        assert!(alignment_lost);
        assert!(chunks.iter().all(Vec::is_empty));
    }

    #[test]
    fn native_tool_formats_survive_prefilter_but_plain_catalogs_do_not() {
        for line in [
            r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"await tools.exec_command({cmd: 'cat /skills/research/SKILL.md'})"}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"read","arguments":{"path":"/skills/research/SKILL.md"}}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Skill","input":{"skill":"research"}}]}}"#,
        ] {
            assert!(has_usage_candidate(line.as_bytes(), None));
        }
        assert!(!has_usage_candidate(
            br#"{"type":"message","role":"user","text":"Please read /skills/research/SKILL.md"}"#,
            None
        ));
    }
}
