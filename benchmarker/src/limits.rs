//! Reading what the process itself is allowed to do.
//!
//! The limits are read from `/proc/self/limits`, so the benchmarker can show
//! exactly what the panel granted it, next to what it actually managed to
//! use.

use std::collections::BTreeMap;

/// The limits of the running process.
#[derive(Debug, Clone, Default)]
pub struct Limits {
    /// The soft and hard value of every limit that was read.
    pub values: BTreeMap<String, (String, String)>,
}

impl Limits {
    /// Read the limits of the running process.
    pub fn read() -> Limits {
        let mut values = BTreeMap::new();
        let Ok(text) = std::fs::read_to_string("/proc/self/limits") else {
            return Limits { values };
        };

        for line in text.lines().skip(1) {
            // The lines look like:
            // Max cpu time              unlimited            unlimited            seconds
            let words: Vec<&str> = line.split_whitespace().collect();
            // The name of the limit is made of words, and is followed by the
            // soft value, the hard value and sometimes a unit. The values are
            // a number or the word unlimited, so the name ends where they
            // begin.
            let Some(start) = words
                .iter()
                .position(|word| *word == "unlimited" || word.parse::<u64>().is_ok())
            else {
                continue;
            };
            if start == 0 || words.len() < start + 2 {
                continue;
            }
            let name = words[..start].join(" ");
            let soft = words[start].to_string();
            let hard = words[start + 1].to_string();
            values.insert(name, (soft, hard));
        }

        Limits { values }
    }
}

/// The memory the process uses right now, in kilobytes.
pub fn memory_kb() -> u64 {
    let Ok(statm) = std::fs::read_to_string("/proc/self/statm") else {
        return 0;
    };
    let Some(pages) = statm.split_whitespace().nth(1) else {
        return 0;
    };
    let Ok(pages) = pages.parse::<u64>() else {
        return 0;
    };
    // The second field of statm is the number of pages that are resident.
    pages * 4
}

/// The processor time the process has used, in seconds.
pub fn cpu_seconds() -> f64 {
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
        return 0.0;
    };
    // The name of the process is in brackets and may contain spaces, so the
    // fields are counted from the closing bracket.
    let Some(end) = stat.rfind(')') else {
        return 0.0;
    };
    let fields: Vec<&str> = stat[end + 1..].split_whitespace().collect();
    // After the name come the state, the parent and a few more, and then the
    // times: utime is the twelfth field of the whole line.
    let utime = fields
        .get(11)
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    let stime = fields
        .get(12)
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    // The clock ticks are a hundred per second on Android.
    (utime + stime) / 100.0
}
