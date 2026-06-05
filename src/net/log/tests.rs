use std::time::{Duration, Instant};

use super::NetworkLog;

#[test]
fn network_log_records_elapsed_messages() {
    let start = Instant::now();
    let mut log = NetworkLog::new(start, 4);

    log.push(start + Duration::from_millis(12), "connected");

    assert_eq!(
        log.entries().collect::<Vec<_>>(),
        vec!["+    12ms connected"]
    );
}

#[test]
fn network_log_respects_capacity() {
    let start = Instant::now();
    let mut log = NetworkLog::new(start, 2);

    log.push(start, "one");
    log.push(start, "two");
    log.push(start, "three");

    assert_eq!(
        log.entries().collect::<Vec<_>>(),
        vec!["+     0ms two", "+     0ms three"]
    );
}
