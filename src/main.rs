#![forbid(unsafe_code)]

mod battery;
mod history;
mod ui;

use battery::{find_battery, Battery};
use history::History;
use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};
use ui::draw;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

const POWER_SUPPLY: &str = "/sys/class/power_supply";
const INTERVAL: Duration = Duration::from_secs(5);
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn sample(root: &Path, history: &mut History) -> Result<Option<Battery>, String> {
    let battery = find_battery(root);
    match &battery {
        Ok(Some(b)) => history.record(b, Instant::now()),
        _ => *history = History::default(),
    }
    battery
}

fn run() -> io::Result<()> {
    enable_raw_mode()?;
    let _guard = TerminalGuard;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut history = History::default();
    let mut battery = sample(Path::new(POWER_SUPPLY), &mut history);
    let mut next_sample = Instant::now() + INTERVAL;
    loop {
        terminal.draw(|frame| draw(frame, &battery, &history))?;
        if event::poll(next_sample.saturating_duration_since(Instant::now()))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Char('c')
                            if key.code == KeyCode::Char('q')
                                || key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            break
                        }
                        KeyCode::Char('r') => {
                            history = History::default();
                            if let Ok(Some(b)) = &battery {
                                history.record(b, Instant::now());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        if Instant::now() >= next_sample {
            battery = sample(Path::new(POWER_SUPPLY), &mut history);
            next_sample = Instant::now() + INTERVAL;
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));
    run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn sampling_clears_history_for_absent_battery_and_root_error() {
        let root = std::env::temp_dir().join(format!("batatui-sampling-{}", std::process::id()));
        let battery_dir = root.join("BAT0");
        fs::create_dir_all(&battery_dir).unwrap();
        fs::write(battery_dir.join("type"), "Battery").unwrap();
        fs::write(battery_dir.join("capacity"), "50").unwrap();
        let mut history = History::default();
        assert!(matches!(sample(&root, &mut history), Ok(Some(_))));
        assert_eq!(history.chart_points(10), vec![50]);
        fs::write(battery_dir.join("present"), "0").unwrap();
        assert!(matches!(sample(&root, &mut history), Ok(None)));
        assert!(history.chart_points(10).is_empty());
        fs::remove_dir_all(&root).unwrap();
        assert!(sample(&root, &mut history).is_err());
        assert!(history.chart_points(10).is_empty());
    }
}
