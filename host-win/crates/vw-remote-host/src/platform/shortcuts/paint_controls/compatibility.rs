//! Pure validation of observed Paint facts. These predicates grant no route,
//! profile, input or editor-effect authority and perform no provider work.

pub(super) fn numeric_range_valid(values: [f64; 5]) -> bool {
    let [current, minimum, maximum, small_change, large_change] = values;
    values
        .iter()
        .all(|value| value.is_finite() && value.abs() <= 1_000_000.0)
        && minimum <= maximum
        && (minimum..=maximum).contains(&current)
        && small_change >= 0.0
        && large_change >= 0.0
}

pub(super) fn observation_clock_valid(start: u64, end: u64) -> bool {
    start != 0 && start <= end && end.checked_mul(100).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_size_range_is_not_a_pixel_mapping_or_a_writability_claim() {
        assert!(numeric_range_valid([2.0, 0.0, 90.0, 1.0, 10.0]));
        // A static/unknown step may be observed. The admitted Size route
        // separately requires its reviewed, writable, positive-step shape.
        assert!(numeric_range_valid([1.0, 1.0, 1.0, 0.0, 0.0]));
    }

    #[test]
    fn invalid_provider_range_never_becomes_a_known_control() {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1_000_001.0] {
            for field in 0..5 {
                let mut values = [2.0, 0.0, 90.0, 1.0, 10.0];
                values[field] = invalid;
                assert!(!numeric_range_valid(values));
            }
        }
        for values in [
            [-1.0, 0.0, 90.0, 1.0, 10.0],
            [91.0, 0.0, 90.0, 1.0, 10.0],
            [2.0, 90.0, 0.0, 1.0, 10.0],
            [2.0, 0.0, 90.0, -1.0, 10.0],
            [2.0, 0.0, 90.0, 1.0, -1.0],
        ] {
            assert!(!numeric_range_valid(values));
        }
    }

    #[test]
    fn frame_clock_preserves_order_and_nanosecond_conversion() {
        let last = u64::MAX / 100;
        assert!(observation_clock_valid(1, 1));
        assert!(observation_clock_valid(last - 1, last));
        assert!(!observation_clock_valid(0, 1));
        assert!(!observation_clock_valid(2, 1));
        assert!(!observation_clock_valid(1, last + 1));
    }
}
