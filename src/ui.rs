use crate::{battery::Battery, history::History};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Wrap},
};
use std::time::Duration;
fn duration(d: Duration) -> String {
    let minutes = d.as_secs() / 60 + u64::from(d.as_secs() % 60 >= 30);
    format!("{}h {:02}m", minutes / 60, minutes % 60)
}

pub(crate) fn draw(
    frame: &mut ratatui::Frame,
    battery: &Result<Option<Battery>, String>,
    history: &History,
) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(9),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(frame.area());
    let gauge = match battery {
        Ok(Some(b)) => Gauge::default()
            .block(
                Block::default()
                    .title(format!(" {} · {} ", b.name, b.status))
                    .borders(Borders::ALL),
            )
            .gauge_style(Style::default().fg(if b.percent < 20.0 {
                Color::Red
            } else {
                Color::Green
            }))
            .ratio((b.percent / 100.0).clamp(0.0, 1.0))
            .label(format!("{:.0}%", b.percent)),
        _ => Gauge::default()
            .block(Block::default().title(" Battery ").borders(Borders::ALL))
            .ratio(0.0)
            .label("Unavailable"),
    };
    frame.render_widget(gauge, areas[0]);
    let mut forecast: Option<(Duration, bool, f64)> = None;
    let details = match battery {
        Ok(Some(b)) => {
            let stored = b
                .stored
                .map(|(now, full, unit)| format!("{now:.2} / {full:.2} {unit}"))
                .unwrap_or_else(|| "unavailable".into());
            let voltage = b
                .voltage_v
                .map(|v| format!("{v:.2} V"))
                .unwrap_or_else(|| "unavailable".into());
            let power = b
                .power_w
                .map(|p| format!("{p:.2} W (device reading)"))
                .unwrap_or_else(|| "unavailable".into());
            let observed = history.observed();
            let selected = observed
                .and_then(|o| {
                    o.trend.map(|trend| {
                        (
                            trend,
                            o.prediction,
                            o.rising,
                            format!("Observed net over {:.1}s (estimate)", o.span.as_secs_f64()),
                        )
                    })
                })
                .or_else(|| {
                    b.snapshot.map(|(trend, prediction, rising)| {
                        (
                            trend,
                            prediction,
                            rising,
                            "Provisional snapshot; assumes current rate".into(),
                        )
                    })
                });
            let (source, trend, prediction) =
                if let Some((rate, predicted, rising, source)) = selected {
                    forecast = predicted.map(|d| (d, rising, b.percent));
                    let prediction = predicted
                        .map(|d| {
                            format!(
                                "~{} until {}{}",
                                duration(d),
                                if rising { "full" } else { "empty" },
                                if areas[2].width < 6 || areas[2].height < 4 {
                                    " (chart too small)"
                                } else {
                                    ""
                                }
                            )
                        })
                        .unwrap_or_else(|| "unavailable".into());
                    (
                        source,
                        format!("{rate:+.2} percentage points/hour"),
                        prediction,
                    )
                } else {
                    (
                        "No usable observed trend or snapshot".into(),
                        "unknown".into(),
                        "unknown".into(),
                    )
                };
            vec![
                Line::from(format!("Stored: {stored}")),
                Line::from(format!("Voltage: {voltage}    Power: {power}")),
                Line::from(format!("Source: {source}")),
                Line::from(format!("Trend: {trend}")),
                Line::from(format!("Prediction: {prediction}")),
            ]
        }
        Ok(None) => vec![Line::from(
            "No readable battery found. Retrying automatically.",
        )],
        Err(e) => vec![
            Line::from(e.as_str()),
            Line::from("Retrying automatically."),
        ],
    };
    frame.render_widget(
        Paragraph::new(details)
            .block(Block::default().title(" Readings ").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        areas[1],
    );
    draw_chart(frame, areas[2], history, forecast);
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::raw(
            "q / Ctrl-C: quit  ·  r: reset history  ·  samples every 5s (session only)",
        )]))
        .wrap(Wrap { trim: true }),
        areas[3],
    );
}

fn draw_chart(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    history: &History,
    forecast: Option<(Duration, bool, f64)>,
) {
    let title = Line::from(vec![
        Span::styled("History", Style::default().fg(Color::Cyan)),
        Span::raw(" | "),
        Span::styled("Forecast", Style::default().fg(Color::Yellow)),
    ]);
    let block = Block::default().title(title).borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height < 2 {
        return;
    }
    let width = usize::from(inner.width);
    let height = inner.height - 1;
    let drawable = forecast.filter(|(d, _, _)| !d.is_zero() && width >= 4);
    let mut samples = history.timed_points();
    let Some((latest, _)) = samples.next_back() else {
        frame.render_widget(
            Paragraph::new("No history"),
            ratatui::layout::Rect::new(inner.x, inner.y + height, inner.width, 1),
        );
        return;
    };
    let oldest = history.timed_points().next().map(|p| p.0).unwrap_or(latest);
    let elapsed = latest.saturating_duration_since(oldest).as_secs_f64();
    let history_columns = if drawable.is_some() {
        (width * 2 / 3).max(2)
    } else {
        width
    };
    let scale = (elapsed / history_columns.saturating_sub(1).max(1) as f64).max(5.0);
    let now_col = ((elapsed / scale).round() as usize).min(width - 1);
    let future_horizon = ((width - 1) as f64 * scale - elapsed).max(0.0);
    let clipped = drawable
        .map(|(d, _, _)| d.as_secs_f64() > future_horizon)
        .unwrap_or(false);
    let next_offset = ((now_col + 1) as f64 * scale - elapsed).max(0.0);
    let status = match forecast {
        None => "",
        Some((d, _, _)) if d.is_zero() => " forecast now",
        Some(_) if width < 4 => " forecast: no space",
        Some((d, _, _)) if d.as_secs_f64() < next_offset => " forecast <1 col",
        Some(_) if clipped => " forecast clipped",
        _ => "",
    };
    let label = format!(
        "{}~{scale:.2}s/col now~{}",
        if status.is_empty() {
            String::new()
        } else {
            format!("{}; ", status.trim())
        },
        now_col + 1
    );
    frame.render_widget(
        Paragraph::new(label),
        ratatui::layout::Rect::new(inner.x, inner.y + height, inner.width, 1),
    );
    let plot = |frame: &mut ratatui::Frame, x: usize, value: f64, color: Color| {
        let units = (value.clamp(0.0, 100.0) / 100.0 * f64::from(height) * 8.0).round() as u16;
        let blocks = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
        for row in 0..height {
            let bottom = height - 1 - row;
            let level = units.saturating_sub(row * 8).min(8) as usize;
            let symbol = if units == 0 && row == 0 {
                "_"
            } else {
                blocks[level]
            };
            frame.buffer_mut()[(inner.x + x as u16, inner.y + bottom)]
                .set_symbol(symbol)
                .set_fg(color);
        }
    };
    for (at, value) in history.timed_points() {
        let since_start = at.saturating_duration_since(oldest).as_secs_f64();
        let col = (since_start / scale).round() as usize;
        if col <= now_col {
            plot(frame, col, value, Color::Cyan);
        }
    }
    if let Some((duration, rising, start)) = drawable {
        let seconds = duration.as_secs_f64();
        let target = if rising { 100.0 } else { 0.0 };
        for x in now_col + 1..width {
            let elapsed = x as f64 * scale - elapsed;
            if elapsed > seconds {
                break;
            }
            plot(
                frame,
                x,
                start + (target - start) * elapsed / seconds,
                Color::Yellow,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use std::time::Instant;
    #[test]
    fn chart_keeps_decreases_and_scrolls_to_latest_samples() {
        let mut b = Battery {
            name: "BAT0".into(),
            status: "Discharging".into(),
            percent: 90.0,
            voltage_v: None,
            power_w: None,
            stored: None,
            snapshot: None,
        };
        let mut h = History::default();
        let now = Instant::now();
        for i in 0..21 {
            b.percent = 90.0 - i as f64;
            h.record(&b, now + Duration::from_secs(i * 5));
        }
        assert_eq!(h.chart_points(3), vec![72, 71, 70]);
        let backend = ratatui::backend::TestBackend::new(20, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw(frame, &Ok(Some(b)), &h))
            .unwrap();
        assert!(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|cell| cell.symbol() == "█" && cell.fg == Color::Cyan));
    }

    #[test]
    fn display_prefers_observed_drain_over_charging_snapshot() {
        let start = Instant::now();
        let b = Battery {
            name: "BAT0".into(),
            status: "Charging".into(),
            percent: 75.0,
            voltage_v: None,
            power_w: None,
            stored: Some((3000.0, 4000.0, "mAh")),
            snapshot: Some((50.0, Some(Duration::from_secs(1800)), true)),
        };
        let mut history = History::default();
        history.record(&b, start);
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| draw(frame, &Ok(Some(b)), &history))
            .unwrap();
        let screen = |terminal: &Terminal<ratatui::backend::TestBackend>| {
            let buffer = terminal.backend().buffer();
            (0..24)
                .flat_map(|y| (0..100).map(move |x| buffer[(x, y)].symbol()))
                .collect::<String>()
        };
        assert!(screen(&terminal).contains("Provisional snapshot; assumes current rate"));
        assert!(screen(&terminal).contains("+50.00 percentage points/hour"));
        assert!(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|c| c.fg == Color::Yellow && c.symbol() == "█"));
        assert_eq!(terminal.backend().buffer()[(11, 12)].fg, Color::Yellow);
        let b = Battery {
            name: "BAT0".into(),
            status: "Charging".into(),
            percent: 75.0,
            voltage_v: None,
            power_w: None,
            stored: Some((3000.0, 4000.0, "mAh")),
            snapshot: Some((50.0, Some(Duration::from_secs(1800)), true)),
        };
        history.record(&b, start + Duration::from_secs(120));
        terminal
            .draw(|frame| draw(frame, &Ok(Some(b)), &history))
            .unwrap();
        assert!(screen(&terminal).contains("Provisional snapshot; assumes current rate"));
        let b = Battery {
            name: "BAT0".into(),
            status: "Charging".into(),
            percent: 74.0,
            voltage_v: None,
            power_w: None,
            stored: Some((2960.0, 4000.0, "mAh")),
            snapshot: Some((50.0, Some(Duration::from_secs(1800)), true)),
        };
        history.record(&b, start + Duration::from_secs(180));
        terminal
            .draw(|frame| draw(frame, &Ok(Some(b)), &history))
            .unwrap();
        assert!(screen(&terminal).contains("Observed net over 180.0s (estimate)"));
        assert!(screen(&terminal).contains("-20.00 percentage points/hour"));
        assert!(!screen(&terminal).contains("+50.00"));
        let b = Battery {
            name: "BAT0".into(),
            status: "Charging".into(),
            percent: 75.0,
            voltage_v: None,
            power_w: None,
            stored: Some((2900.0, 4000.0, "mAh")),
            snapshot: Some((50.0, Some(Duration::from_secs(1800)), true)),
        };
        history.record(&b, start + Duration::from_secs(240));
        terminal
            .draw(|frame| draw(frame, &Ok(Some(b)), &history))
            .unwrap();
        let screen = screen(&terminal);
        assert!(screen.contains("Observed net over 240.0s (estimate)"));
        assert!(screen.contains("-37.50 percentage points/hour"));
        assert!(screen.contains("until empty"));
        assert!(!screen.contains("until full"));
    }

    #[test]
    fn chart_uses_two_colors_and_shared_axis() {
        let mut b = Battery {
            name: "BAT0".into(),
            status: "Charging".into(),
            percent: 90.0,
            voltage_v: None,
            power_w: None,
            stored: None,
            snapshot: None,
        };
        let mut history = History::default();
        let now = Instant::now();
        history.record(&b, now);
        b.percent = 50.0;
        history.record(&b, now + Duration::from_secs(1));
        history.record(&b, now + Duration::from_secs(20));
        let render = |forecast| {
            let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(42, 12)).unwrap();
            terminal
                .draw(|frame| draw_chart(frame, frame.area(), &history, forecast))
                .unwrap();
            terminal.backend().buffer().clone()
        };
        let chart = render(Some((Duration::from_secs(3600), true, 50.0)));
        assert_eq!(chart[(1, 9)].fg, Color::Cyan);
        assert_eq!(chart[(1, 1)].symbol(), " ");
        assert_eq!(chart[(5, 9)].fg, Color::Cyan);
        assert_eq!(chart[(2, 9)].symbol(), " ");
        assert_eq!(chart[(6, 9)].symbol(), "█");
        assert_eq!(chart[(6, 9)].fg, Color::Yellow);
        assert_eq!(chart[(7, 9)].fg, Color::Yellow);
        let label: String = (1..41).map(|x| chart[(x, 10)].symbol()).collect();
        assert!(label.contains("5.00s/col now~5"));
        assert!(label.contains("forecast clipped"));
        let absent = render(None);
        let zero = render(Some((Duration::ZERO, true, 50.0)));
        assert_eq!(absent[(11, 0)].fg, Color::Yellow);
        let subcolumn = render(Some((Duration::from_secs(1), true, 50.0)));
        let sublabel: String = (1..41).map(|x| subcolumn[(x, 10)].symbol()).collect();
        assert!(sublabel.contains("forecast <1 col"));
        assert_eq!(subcolumn[(11, 0)].fg, Color::Yellow);
        let zerolabel: String = (1..41).map(|x| zero[(x, 10)].symbol()).collect();
        assert!(zerolabel.contains("forecast now"));
        assert!(!(1..10).any(|y| (1..41).any(|x| absent[(x, y)].fg == Color::Yellow)));
        assert!(!(1..10).any(|y| (1..41).any(|x| zero[(x, y)].fg == Color::Yellow)));
        let falling = render(Some((Duration::from_secs(10), false, 50.0)));
        assert_eq!(falling[(7, 9)].symbol(), "_");
        assert_eq!(falling[(7, 9)].fg, Color::Yellow);
    }

    #[test]
    fn chart_starts_left_for_short_and_long_histories() {
        let b = Battery {
            name: "BAT0".into(),
            status: "Charging".into(),
            percent: 50.0,
            voltage_v: None,
            power_w: None,
            stored: None,
            snapshot: None,
        };
        let now = Instant::now();
        let mut history = History::default();
        history.record(&b, now);
        let render = |history: &History, forecast, width| {
            let mut terminal =
                Terminal::new(ratatui::backend::TestBackend::new(width, 12)).unwrap();
            terminal
                .draw(|frame| draw_chart(frame, frame.area(), history, forecast))
                .unwrap();
            terminal.backend().buffer().clone()
        };
        let short = render(&history, Some((Duration::from_secs(300), true, 50.0)), 42);
        assert_eq!(short[(1, 9)].fg, Color::Cyan);
        assert_eq!(short[(2, 9)].fg, Color::Yellow);
        let short_without = render(&history, None, 42);
        assert_eq!(short_without[(1, 9)].fg, Color::Cyan);
        history.record(&b, now + Duration::from_secs(7));
        let rounded = render(&history, Some((Duration::from_secs(3), false, 50.0)), 42);
        assert_eq!(rounded[(2, 9)].fg, Color::Cyan);
        assert_eq!(rounded[(3, 9)].symbol(), "_");
        assert_eq!(rounded[(3, 9)].fg, Color::Yellow);
        history.record(&b, now + Duration::from_secs(400));
        let long = render(&history, Some((Duration::from_secs(3600), true, 50.0)), 42);
        assert_eq!(long[(1, 9)].fg, Color::Cyan);
        assert_eq!(long[(26, 9)].fg, Color::Cyan);
        assert_eq!(long[(27, 9)].fg, Color::Yellow);
        let long_without = render(&history, None, 42);
        assert_eq!(long_without[(1, 9)].fg, Color::Cyan);
        assert_eq!(long_without[(40, 9)].fg, Color::Cyan);
        for width in 1..=4 {
            render(&history, Some((Duration::from_secs(10), true, 50.0)), width);
        }
    }
}
