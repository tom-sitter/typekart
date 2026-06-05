//! Shared tick-boundary helpers.
//!
//! Adapters own clocks and scheduling. These helpers only decide how much
//! elapsed time a tick should process once an adapter supplies timestamps.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundedTick {
    pub elapsed_ms: f64,
    pub accepted_at_ms: Option<f64>,
}

pub fn bounded_tick_elapsed_ms(
    previous_tick_ms: Option<f64>,
    now_ms: f64,
    configured_tick_ms: u32,
    minimum_elapsed_ratio: f64,
    maximum_elapsed_ms: f64,
) -> Option<BoundedTick> {
    let Some(previous_tick_ms) = previous_tick_ms else {
        return Some(BoundedTick {
            elapsed_ms: f64::from(configured_tick_ms),
            accepted_at_ms: None,
        });
    };

    let elapsed_ms = now_ms - previous_tick_ms;
    let minimum_elapsed_ms = f64::from(configured_tick_ms) * minimum_elapsed_ratio;
    if elapsed_ms < minimum_elapsed_ms {
        return None;
    }

    Some(BoundedTick {
        elapsed_ms: elapsed_ms.min(maximum_elapsed_ms),
        accepted_at_ms: Some(now_ms),
    })
}

#[cfg(test)]
mod tests {
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
}
