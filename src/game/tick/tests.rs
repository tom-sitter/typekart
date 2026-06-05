use super::{BoundedTick, bounded_tick_elapsed_ms};

#[test]
fn first_tick_uses_configured_elapsed_without_accepting_clock() {
    assert_eq!(
        bounded_tick_elapsed_ms(None, 1_000.0, 250, 0.5, 1_000.0),
        Some(BoundedTick {
            elapsed_ms: 250.0,
            accepted_at_ms: None,
        })
    );
}

#[test]
fn early_tick_is_ignored() {
    assert_eq!(
        bounded_tick_elapsed_ms(Some(1_000.0), 1_100.0, 250, 0.5, 1_000.0),
        None
    );
}

#[test]
fn accepted_tick_is_capped_and_records_clock() {
    assert_eq!(
        bounded_tick_elapsed_ms(Some(1_000.0), 2_500.0, 250, 0.5, 1_000.0),
        Some(BoundedTick {
            elapsed_ms: 1_000.0,
            accepted_at_ms: Some(2_500.0),
        })
    );
}
