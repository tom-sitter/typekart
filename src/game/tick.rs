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
mod tests;
