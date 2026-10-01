//! Parsing coverage for process memory sampling.

use super::*;

/// Both values are converted from kB to MiB.
#[test]
fn parses_rss_and_peak() {
    let status = "Name:\tcsaf-trove-serv\nVmPeak:\t 3601340 kB\nVmHWM:\t 3084512 kB\nVmRSS:\t 2970324 kB\nThreads:\t104\n";
    assert_eq!(
        parse(status),
        Some(MemoryUsage {
            rss_mib: 2900,
            peak_mib: 3012,
        })
    );
}

/// Missing or malformed fields yield no sample instead of misleading zeros.
#[test]
fn rejects_incomplete_status() {
    assert_eq!(parse("VmRSS:\t 1024 kB\n"), None);
    assert_eq!(parse("VmRSS:\t abc kB\nVmHWM:\t 1024 kB\n"), None);
}
