//! Frozen local UTF-8 byte measurements, counted once per exposed placement.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::scan::{ScanResult, SkillPlacement};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntrypointBytes {
    pub metadata_bytes: u64,
    pub frontmatter_bytes: Option<u64>,
}

impl EntrypointBytes {
    pub(crate) fn measure(content: &str) -> Self {
        let mut lines = content.split_inclusive('\n');
        let first = lines.next().unwrap_or_default();
        let frontmatter_bytes = if first.trim_end_matches(['\r', '\n']) == "---" {
            let mut bytes = first.len();
            lines.find_map(|line| {
                bytes += line.len();
                (line.trim_end_matches(['\r', '\n']) == "---").then_some(bytes as u64)
            })
        } else {
            Some(0)
        };
        Self {
            metadata_bytes: content.len() as u64,
            frontmatter_bytes,
        }
    }
}

#[derive(Default, Serialize)]
pub struct ExposedBytes {
    pub default_exposed_placement_count: usize,
    pub exposed_metadata_bytes: u64,
    pub exposed_frontmatter_bytes: u64,
    pub metadata_unknown_placement_count: usize,
    pub frontmatter_unknown_placement_count: usize,
}

impl ExposedBytes {
    fn add(&mut self, measurement: Option<&EntrypointBytes>) {
        self.default_exposed_placement_count += 1;
        if let Some(bytes) = measurement {
            self.exposed_metadata_bytes += bytes.metadata_bytes;
            if let Some(bytes) = bytes.frontmatter_bytes {
                self.exposed_frontmatter_bytes += bytes;
            } else {
                self.frontmatter_unknown_placement_count += 1;
            }
        } else {
            self.metadata_unknown_placement_count += 1;
            self.frontmatter_unknown_placement_count += 1;
        }
    }
}

pub fn summarize<'a>(
    scan: &ScanResult,
    placements: impl IntoIterator<Item = &'a SkillPlacement>,
) -> serde_json::Value {
    let mut totals = ExposedBytes::default();
    let mut agents = BTreeMap::<&str, ExposedBytes>::new();
    for placement in placements {
        let measurement = scan.entrypoint_bytes.get(&placement.id);
        totals.add(measurement);
        if let Some(agent) = placement.agent {
            agents.entry(agent.id()).or_default().add(measurement);
        }
    }
    let mut result = serde_json::to_value(totals).expect("serialize byte counts");
    result["by_agent"] = serde_json::json!(agents);
    result["basis"] = serde_json::json!("local_disk_utf8_bytes_per_default_exposed_placement");
    result
}

/// Summary views keep all totals; full Scan/Report and Plans retain per-Agent detail.
pub fn compact(mut footprint: serde_json::Value) -> serde_json::Value {
    if let Some(object) = footprint.as_object_mut() {
        object.remove("by_agent");
    }
    footprint
}

pub fn exposed(scan: &ScanResult) -> serde_json::Value {
    summarize(scan, scan.placements.iter().filter(|p| p.default_exposed))
}
