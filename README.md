# batatui

A Linux terminal monitor for battery charge, usage, trends and predictions, built with [ratatui](https://ratatui.rs/).

## Run

Install Rust and Cargo, then run in an interactive terminal:

```sh
cargo run --release
```

Press `q` or `Ctrl-C` to quit; press `r` to clear the history. The terminal is restored on exit. A missing or unreadable battery is shown as unavailable and retried automatically. The monitor selects the first readable battery in alphabetical sysfs order; it does not aggregate multiple batteries.

## Readings and estimates

Linux power-supply sysfs is sampled every five seconds. Charge is shown in mAh when the device exposes charge, or Wh when it exposes energy. Power is the device's reported power, or current multiplied by voltage when those are available; it is a magnitude, with direction indicated by the device status. Fields not supplied by the hardware are unavailable.

History is held **only in memory for the current session**, with at most 720 samples (about one hour). It resets when the selected battery changes, when the battery becomes unavailable, or when you press `r`; changes in charge or status do not clear it. The chart uses a fixed 0–100% scale and scrolls to show the newest samples when history exceeds its width.

The chart starts the retained history at the left, using cyan bars for measurements and yellow bars for the selected projection. Both share the labeled approximate seconds-per-column scale and `now` column. Measured timestamps determine sample positions, so gaps remain visible where resolution permits; when samples share a column, the newest sample is shown. The scale expands with history, while long projections are clipped at the chart edge and labeled as such. The projection uses actual elapsed time from the latest measurement, interpolating from the displayed (possibly rounded) percentage toward full or empty over the estimated duration; an underscore marks zero. Because the percentage and the raw quantities used for estimates may differ, the plotted slope is approximate.

The yellow forecast legend is visible even while no prediction is available. If a valid prediction ends before the next chart column, the axis says `forecast <1 col` instead of drawing a misleading bar; a zero-duration prediction is labeled `forecast now`. A very small chart may not have room for forecast bars.

Trend and prediction prefer **observed net change** between the oldest and newest compatible samples, across charging/discharging status changes. Any nonzero change over nonzero elapsed time is used, so quantized or noisy readings and short intervals can produce highly inaccurate estimates. Energy or charge readings are used when available, otherwise rounded percentage is used. Unit or measurement-source changes start a new observation window without clearing the chart. The observed direction, not the latest status, determines whether the prediction is to full or empty.

When observed change is unavailable or flat, the display uses a **provisional snapshot** if current and charge, or power and energy, are available with a nonzero rate and an active charging/discharging status. It assumes the current rate persists; it is not evidence of a sustained net trend, and sysfs values can change while being read. Charge-based estimates do not require voltage. Without a usable observation or snapshot, trend and prediction are unknown. Both approaches extrapolate a rate unchanged and can be highly inaccurate; flat readings do not prove zero drift.

## Development

- `src/main.rs` owns the event loop and terminal lifecycle. Its shared `sample` function handles both the initial and periodic sysfs reads and updates or clears history; manual reset reuses the cached reading.
- `src/battery.rs` discovers batteries and reads sysfs values.
- `src/history.rs` retains chart samples and calculates observed net estimates.
- `src/ui.rs` renders the readings and chart.
