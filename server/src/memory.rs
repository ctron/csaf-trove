//! Process memory sampling for attributing peaks to pipeline phases.

/// Resident memory of the current process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryUsage {
    /// Current resident set size in MiB.
    pub rss_mib: u64,
    /// Peak resident set size since process start in MiB.
    pub peak_mib: u64,
}

/// Reads the current memory usage, or `None` where `/proc/self/status` is unavailable.
pub fn usage() -> Option<MemoryUsage> {
    parse(&std::fs::read_to_string("/proc/self/status").ok()?)
}

/// Extracts `VmRSS` and `VmHWM` from a `/proc/<pid>/status` document.
fn parse(status: &str) -> Option<MemoryUsage> {
    let kib = |key: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))?
            .trim()
            .strip_suffix("kB")?
            .trim()
            .parse::<u64>()
            .ok()
    };
    Some(MemoryUsage {
        rss_mib: kib("VmRSS:")? / 1024,
        peak_mib: kib("VmHWM:")? / 1024,
    })
}

#[cfg(test)]
mod tests;
