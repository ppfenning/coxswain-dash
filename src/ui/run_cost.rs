//! Pure chart helpers for the run cost frame. Plain data in, numbers and indexes out: time is
//! epoch seconds as `f64`, cost is dollars as `f64`. No feed types, no theme, no widget, no clock.
#![allow(dead_code)]

/// Axis extent for the cost chart. The y axis always starts at 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x_min: f64,
    pub x_max: f64,
    pub y_max: f64,
}

/// Rounds up to the next of 1, 2, 2.5, 5 or 10 times a power of ten; non-positive input gives 1.
fn round_up_step(y: f64) -> f64 {
    if y <= 0.0 {
        1.0
    } else {
        let magnitude = 10f64.powi(y.log10().floor() as i32);
        [1.0, 2.0, 2.5, 5.0, 10.0]
            .iter()
            .map(|f| f * magnitude)
            .find(|top| y <= *top)
            .unwrap_or(10.0 * magnitude)
    }
}

/// Bounds covering every point of every series and of spend; None when all are empty.
pub fn chart_bounds(series: &[Vec<(f64, f64)>], spend: &[(f64, f64)]) -> Option<Bounds> {
    let points = || series.iter().flatten().chain(spend.iter());
    points().next()?;
    let (x_min, x_max, y_top) = points().fold(
        (f64::INFINITY, f64::NEG_INFINITY, 0.0_f64),
        |(lo, hi, top), (x, y)| (lo.min(*x), hi.max(*x), top.max(*y)),
    );
    Some(Bounds {
        x_min,
        x_max: if x_max > x_min { x_max } else { x_min + 1.0 },
        y_max: round_up_step(y_top),
    })
}

/// `count` evenly spaced labels from $0 to `y_max`; a whole value drops its decimals, any other gets two.
pub fn dollar_labels(y_max: f64, count: usize) -> Vec<String> {
    let label = |v: f64| {
        if (v - v.round()).abs() < 1e-9 {
            format!("${}", v.round())
        } else {
            format!("${v:.2}")
        }
    };
    match count {
        0 => Vec::new(),
        1 => vec![label(0.0)],
        n => (0..n)
            .map(|i| label(y_max * i as f64 / (n - 1) as f64))
            .collect(),
    }
}

/// Indexes of the series that have at least one point.
pub fn drawable(series: &[Vec<(f64, f64)>]) -> Vec<usize> {
    series
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.is_empty())
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_cover_two_series_and_spend() {
        let series = vec![
            vec![(10.0, 0.5), (20.0, 1.5)],
            vec![(5.0, 0.2), (15.0, 3.0)],
        ];
        let spend = [(30.0, 4.0)];
        assert_eq!(
            chart_bounds(&series, &spend),
            Some(Bounds {
                x_min: 5.0,
                x_max: 30.0,
                y_max: 5.0
            })
        );
    }

    #[test]
    fn bounds_are_none_when_everything_is_empty() {
        assert_eq!(chart_bounds(&[vec![], vec![]], &[]), None);
        assert_eq!(chart_bounds(&[], &[]), None);
    }

    #[test]
    fn a_single_point_series_widens_x() {
        let b = chart_bounds(&[vec![(100.0, 1.0)]], &[]).unwrap();
        assert_eq!((b.x_min, b.x_max), (100.0, 101.0));
    }

    #[test]
    fn y_max_stays_on_a_step_boundary() {
        assert_eq!(chart_bounds(&[vec![(0.0, 5.0)]], &[]).unwrap().y_max, 5.0);
    }

    #[test]
    fn y_max_rounds_up_just_above_a_step_boundary() {
        assert_eq!(chart_bounds(&[vec![(0.0, 5.01)]], &[]).unwrap().y_max, 10.0);
    }

    #[test]
    fn labels_use_whole_dollars_from_one_up() {
        assert_eq!(dollar_labels(5.0, 3), ["$0", "$2.50", "$5"]);
    }

    #[test]
    fn labels_use_cents_below_one_dollar() {
        assert_eq!(dollar_labels(0.5, 3), ["$0", "$0.25", "$0.50"]);
    }

    #[test]
    fn drawable_skips_an_empty_series() {
        let series = vec![vec![(0.0, 1.0)], vec![], vec![(0.0, 2.0), (1.0, 3.0)]];
        assert_eq!(drawable(&series), vec![0, 2]);
    }
}
