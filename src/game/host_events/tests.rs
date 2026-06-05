use super::push_capped_event;

#[test]
fn capped_events_keep_recent_entries() {
    let mut events = Vec::new();

    push_capped_event(&mut events, "one", 2);
    push_capped_event(&mut events, "two", 2);
    push_capped_event(&mut events, "three", 2);

    assert_eq!(events, vec!["two".to_string(), "three".to_string()]);
}

#[test]
fn capped_events_ignore_zero_capacity() {
    let mut events = Vec::new();

    push_capped_event(&mut events, "one", 0);

    assert!(events.is_empty());
}
