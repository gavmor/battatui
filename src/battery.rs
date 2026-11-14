use std::{fs, path::{Path, PathBuf}};
use std::time::Duration;

#[derive(Debug)]
pub(crate) struct Battery {
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) percent: f64,
    pub(crate) voltage_v: Option<f64>,
    pub(crate) power_w: Option<f64>,
    pub(crate) stored: Option<(f64, f64, &'static str)>,
    pub(crate) snapshot: Option<(f64, Option<Duration>, bool)>,
}

fn text(path: &Path, name: &str) -> Option<String> {
    fs::read_to_string(path.join(name)).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

fn number(path: &Path, name: &str) -> Option<f64> {
    text(path, name)?.parse::<f64>().ok().filter(|n| n.is_finite())
}

fn read_battery(path: &Path) -> Option<Battery> {
    if text(path, "type").as_deref() != Some("Battery") || number(path, "present") == Some(0.0) {
        return None;
    }
    let energy = number(path, "energy_now").zip(number(path, "energy_full"))
        .filter(|(now, full)| *now >= 0.0 && *full > 0.0)
        .map(|(now, full)| (now / 1_000_000.0, full / 1_000_000.0, "Wh"));
    let charge = number(path, "charge_now").zip(number(path, "charge_full"))
        .filter(|(now, full)| *now >= 0.0 && *full > 0.0)
        .map(|(now, full)| (now / 1_000.0, full / 1_000.0, "mAh"));
    let stored = energy.or(charge);
    let percent = number(path, "capacity").filter(|p| (0.0..=100.0).contains(p))
        .or_else(|| stored.map(|(now, full, _)| (100.0 * now / full).clamp(0.0, 100.0)))?;
    let voltage_v = number(path, "voltage_now").filter(|v| *v > 0.0).map(|v| v / 1_000_000.0);
    // sysfs power is µW; current is µA and voltage is µV. Direction comes from status.
    let current_ma = number(path, "current_now").map(|i| i.abs() / 1_000.0);
    let power_w = number(path, "power_now").map(|p| p.abs() / 1_000_000.0)
        .or_else(|| current_ma.zip(voltage_v).map(|(i, v)| i / 1_000.0 * v));
    let status = text(path, "status").unwrap_or_else(|| "Unknown".into());
    let estimate = |stored: Option<(f64, f64, &'static str)>, rate: Option<f64>| {
        let (now, full, _) = stored?;
        let rate = rate?;
        if !now.is_finite() || !full.is_finite() || full <= 0.0 || !rate.is_finite() || rate <= 0.0 { return None; }
        let rising = match status.as_str() { "Charging" => true, "Discharging" => false, _ => return None };
        let trend = (if rising { 1.0 } else { -1.0 }) * rate / full * 100.0;
        if !trend.is_finite() || trend == 0.0 { return None; }
        let remaining = if rising { (full - now).max(0.0) } else { now };
        Some((trend, Duration::try_from_secs_f64(remaining / rate * 3600.0).ok(), rising))
    };
    let snapshot = estimate(energy, power_w).or_else(|| estimate(charge, current_ma));
    Some(Battery {
        name: path.file_name()?.to_string_lossy().into_owned(),
        status,
        percent, voltage_v, power_w, stored, snapshot,
    })
}


pub(crate) fn find_battery(root: &Path) -> Result<Option<Battery>, String> {
    let entries = fs::read_dir(root).map_err(|e| format!("Cannot read {}: {e}", root.display()))?;
    let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    Ok(paths.iter().find_map(|p| read_battery(p)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("batatui-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn put(&self, name: &str, value: &str) { fs::write(self.0.join(name), value).unwrap(); }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
    #[test]
    fn reads_charge_based_battery_and_converts_units() {
        let f = Fixture::new();
        for (k, v) in [("type", "Battery"), ("status", "Discharging"), ("charge_now", "3000000"), ("charge_full", "6000000"), ("current_now", "-2000000"), ("voltage_now", "12000000")] { f.put(k, v); }
        let b = read_battery(&f.0).unwrap();
        assert_eq!(b.percent, 50.0);
        assert_eq!(b.stored, Some((3000.0, 6000.0, "mAh")));
        assert_eq!(b.power_w, Some(24.0));
    }
    #[test]
    fn reads_energy_and_handles_missing_battery() {
        let f = Fixture::new();
        assert!(read_battery(&f.0).is_none());
        for (k, v) in [("type", "Battery"), ("energy_now", "20000000"), ("energy_full", "40000000"), ("power_now", "5000000")] { f.put(k, v); }
        let b = read_battery(&f.0).unwrap();
        assert_eq!(b.percent, 50.0);
        assert_eq!(b.stored, Some((20.0, 40.0, "Wh")));
        assert_eq!(b.power_w, Some(5.0));
        f.put("present", "0");
        assert!(read_battery(&f.0).is_none());
    }
    #[test]
    fn reads_charge_without_voltage() {
        let f = Fixture::new();
        for (k, v) in [("type", "Battery"), ("capacity", "52"), ("charge_now", "1976000"), ("charge_full", "3800000"), ("current_now", "1900000")] { f.put(k, v); }
        let b = read_battery(&f.0).unwrap();
        assert_eq!(b.percent, 52.0);
        assert_eq!(b.stored, Some((1976.0, 3800.0, "mAh")));
        assert_eq!(b.power_w, None);
    }

    #[test]
    fn snapshot_uses_correct_units_and_requires_active_nonzero_rate() {
        let f = Fixture::new();
        for (k, v) in [("type", "Battery"), ("status", "Charging"), ("capacity", "52"), ("charge_now", "1976000"), ("charge_full", "3800000"), ("current_now", "1900000")] { f.put(k, v); }
        let b = read_battery(&f.0).unwrap();
        assert_eq!(b.snapshot, Some((50.0, Some(Duration::from_secs(3456)), true)));
        f.put("status", "Discharging");
        assert_eq!(read_battery(&f.0).unwrap().snapshot, Some((-50.0, Some(Duration::from_secs(3744)), false)));
        f.put("current_now", "0");
        assert!(read_battery(&f.0).unwrap().snapshot.is_none());
        fs::remove_file(f.0.join("current_now")).unwrap();
        assert!(read_battery(&f.0).unwrap().snapshot.is_none());
        f.put("current_now", "1900000");
        f.put("status", "Full");
        assert!(read_battery(&f.0).unwrap().snapshot.is_none());
        f.put("status", "Charging");
        f.put("energy_now", "20000000");
        f.put("energy_full", "40000000");
        f.put("power_now", "5000000");
        assert_eq!(read_battery(&f.0).unwrap().snapshot, Some((12.5, Some(Duration::from_secs(14400)), true)));
    }

    #[test]
    fn alternating_current_reports_net_loss_even_when_charging() {
        use crate::history::History;
        use std::time::{Duration, Instant};
        let f = Fixture::new();
        for (k, v) in [("type", "Battery"), ("status", "Charging"), ("charge_now", "3000000"), ("charge_full", "3800000"), ("current_now", "1900000")] { f.put(k, v); }
        let start = Instant::now();
        let mut history = History::default();
        history.record(&read_battery(&f.0).unwrap(), start);
        f.put("status", "Discharging");
        f.put("current_now", "500000");
        f.put("charge_now", "2980000");
        history.record(&read_battery(&f.0).unwrap(), start + Duration::from_secs(23));
        f.put("status", "Charging");
        f.put("current_now", "1900000");
        f.put("charge_now", "2900000");
        let battery = read_battery(&f.0).unwrap();
        history.record(&battery, start + Duration::from_secs(120));
        let net = history.observed().unwrap();
        assert!(net.trend.unwrap() < 0.0);
        assert!(!net.rising);
        assert_eq!(net.prediction, Some(Duration::from_secs(3480)));
    }
}
