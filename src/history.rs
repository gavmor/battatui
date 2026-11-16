use crate::battery::Battery;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const HISTORY_LIMIT: usize = 720; // One hour at the normal sampling interval.
struct Sample {
    percent: f64,
    at: Instant,
    measurement: (f64, f64, &'static str),
}

pub(crate) struct Observed {
    pub(crate) trend: Option<f64>,
    pub(crate) prediction: Option<Duration>,
    pub(crate) rising: bool,
    pub(crate) span: Duration,
}

#[derive(Default)]
pub(crate) struct History {
    samples: VecDeque<Sample>,
    battery: String,
}

impl History {
    pub(crate) fn record(&mut self, battery: &Battery, at: Instant) {
        if self.battery != battery.name {
            self.samples.clear();
            self.battery.clone_from(&battery.name);
        }
        let measurement = battery
            .stored
            .filter(|(now, full, _)| {
                now.is_finite() && *now >= 0.0 && full.is_finite() && *full > 0.0
            })
            .unwrap_or((battery.percent, 100.0, "%"));
        self.samples.push_back(Sample {
            percent: battery.percent,
            at,
            measurement,
        });
        if self.samples.len() > HISTORY_LIMIT {
            self.samples.pop_front();
        }
    }

    #[cfg(test)]
    pub(crate) fn chart_points(&self, width: usize) -> Vec<u64> {
        self.samples
            .iter()
            .skip(self.samples.len().saturating_sub(width))
            .map(|s| s.percent.round().clamp(0.0, 100.0) as u64)
            .collect()
    }

    pub(crate) fn timed_points(&self) -> impl DoubleEndedIterator<Item = (Instant, f64)> + '_ {
        self.samples.iter().map(|s| (s.at, s.percent))
    }

    pub(crate) fn observed(&self) -> Option<Observed> {
        let last = self.samples.back()?;
        let (now, full, unit) = last.measurement;
        // Never compare different units, including transitions to or from percentage fallback.
        let first = self
            .samples
            .iter()
            .rev()
            .take_while(|s| s.measurement.2 == unit)
            .last()?;
        let span = last.at.checked_duration_since(first.at)?;
        if span.is_zero() {
            return None;
        }
        let delta = now - first.measurement.0;
        let rate = delta / span.as_secs_f64() * 3600.0;
        let trend = rate / full * 100.0;
        if !trend.is_finite() || trend == 0.0 {
            return None;
        }
        let remaining = if rate > 0.0 {
            (full - now).max(0.0)
        } else {
            now
        };
        let prediction = Duration::try_from_secs_f64(remaining / rate.abs() * 3600.0).ok();
        Some(Observed {
            trend: Some(trend),
            prediction,
            rising: rate > 0.0,
            span,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_survives_status_and_charge_changes() {
        let mut b = Battery {
            name: "BAT0".into(),
            status: "Discharging".into(),
            percent: 60.0,
            voltage_v: None,
            power_w: None,
            stored: None,
            snapshot: None,
        };
        let mut h = History::default();
        let now = Instant::now();
        h.record(&b, now);
        b.percent = 59.0;
        h.record(&b, now + Duration::from_secs(5));
        b.status = "Charging".into();
        h.record(&b, now + Duration::from_secs(10));
        b.percent = 60.0;
        h.record(&b, now + Duration::from_secs(15));
        assert_eq!(h.chart_points(10), vec![60, 59, 59, 60]);
    }

    #[test]
    fn observed_net_uses_raw_charge_across_status_switches() {
        let now = Instant::now();
        let mut b = Battery {
            name: "BAT0".into(),
            status: "Charging".into(),
            percent: 79.0,
            voltage_v: None,
            power_w: None,
            stored: Some((3000.0, 3800.0, "mAh")),
            snapshot: None,
        };
        let mut h = History::default();
        h.record(&b, now);
        b.status = "Discharging".into();
        b.stored = Some((2980.0, 3800.0, "mAh"));
        h.record(&b, now + Duration::from_secs(17));
        assert!(h.observed().unwrap().trend.unwrap() < 0.0);
        b.status = "Charging".into();
        b.stored = Some((2900.0, 3800.0, "mAh"));
        h.record(&b, now + Duration::from_secs(120));
        let observed = h.observed().unwrap();
        assert!((observed.trend.unwrap() + 3000.0 / 3800.0 * 100.0).abs() < 1e-10);
        assert!(!observed.rising);
        assert_eq!(observed.prediction, Some(Duration::from_secs(3480)));
        assert_eq!(h.chart_points(10), vec![79, 79, 79]);
    }

    #[test]
    fn observed_handles_energy_growth_zero_and_unit_or_identity_changes() {
        let start = Instant::now();
        let mut b = Battery {
            name: "BAT0".into(),
            status: "Discharging".into(),
            percent: 50.0,
            voltage_v: None,
            power_w: None,
            stored: Some((20.0, 40.0, "Wh")),
            snapshot: None,
        };
        let mut h = History::default();
        h.record(&b, start);
        h.record(&b, start + Duration::from_secs(120));
        assert!(h.observed().is_none());
        b.stored = Some((21.0, 40.0, "Wh"));
        h.record(&b, start + Duration::from_secs(180));
        let rising = h.observed().unwrap();
        assert_eq!(rising.trend, Some(50.0));
        assert!(rising.rising);
        assert_eq!(rising.prediction, Some(Duration::from_secs(3420)));
        b.stored = Some((2100.0, 4000.0, "mAh"));
        h.record(&b, start + Duration::from_secs(185));
        assert!(h.observed().is_none());
        b.stored = None;
        h.record(&b, start + Duration::from_secs(220));
        assert!(h.observed().is_none());
        b.percent = 47.0;
        h.record(&b, start + Duration::from_secs(340));
        assert!(h.observed().unwrap().trend.unwrap() < 0.0);
        b.name = "BAT1".into();
        h.record(&b, start + Duration::from_secs(380));
        assert!(h.observed().is_none());
        assert_eq!(h.chart_points(10), vec![47]);
    }

    #[test]
    fn smallest_observed_change_needs_only_nonzero_time() {
        let start = Instant::now();
        let mut b = Battery {
            name: "BAT0".into(),
            status: "Charging".into(),
            percent: 50.0,
            voltage_v: None,
            power_w: None,
            stored: Some((2000.0, 4000.0, "mAh")),
            snapshot: None,
        };
        let mut h = History::default();
        h.record(&b, start);
        b.stored = Some((1999.999, 4000.0, "mAh"));
        h.record(&b, start);
        assert!(h.observed().is_none());
        h.record(&b, start + Duration::from_secs(1));
        assert!(h.observed().unwrap().trend.unwrap() < 0.0);
    }
}
