//! Drawing. Every frame is drawn from `App` state; effects are applied last.

use std::time::Duration;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use tachyonfx::{Interpolation, Motion, fx};
use vme_core::instruments;
use vme_core::notation::format_length;
use vme_core::theory::{format_cents, note_name};
use vme_core::transcribe::ChangeKind;

use crate::app::{App, Focus, FxKey, FxRequest, Level, Lower, Mode};
use crate::command::COMMANDS;
use crate::smooth::Meter;

pub const BG: Color = Color::Rgb(15, 15, 21);
const PANEL: Color = Color::Rgb(19, 19, 27);
const ROW_IN: Color = Color::Rgb(29, 29, 40);
const ROW_OUT: Color = Color::Rgb(20, 20, 27);
const BEAT: Color = Color::Rgb(36, 36, 50);
const BAR: Color = Color::Rgb(50, 50, 68);
const TEXT: Color = Color::Rgb(222, 222, 232);
const DIM: Color = Color::Rgb(115, 115, 138);
const FAINT: Color = Color::Rgb(70, 70, 90);
const ACCENT: Color = Color::Rgb(255, 176, 64);
const NOTE: Color = Color::Rgb(78, 196, 222);
const NOTE_SEL: Color = Color::Rgb(255, 196, 92);
const RED: Color = Color::Rgb(240, 84, 84);
const GREEN: Color = Color::Rgb(96, 212, 128);
const YELLOW: Color = Color::Rgb(232, 210, 92);

fn scale(c: Color, k: f32) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            (f32::from(r) * k).min(255.0) as u8,
            (f32::from(g) * k).min(255.0) as u8,
            (f32::from(b) * k).min(255.0) as u8,
        ),
        other => other,
    }
}

fn panel(title: &str, focused: bool) -> Block<'_> {
    let border = if focused { ACCENT } else { FAINT };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(PANEL))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(if focused { ACCENT } else { DIM })
                .add_modifier(Modifier::BOLD),
        ))
}

pub fn draw(f: &mut Frame, app: &mut App, dt: Duration) {
    let area = f.area();
    f.buffer_mut()
        .set_style(area, Style::default().bg(BG).fg(TEXT));
    if area.width < 64 || area.height < 18 {
        let p = Paragraph::new(format!(
            "vme needs at least 64×18 characters (now {}×{}).\nMake the terminal bigger, or press q to quit.",
            area.width, area.height
        ))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
        f.render_widget(p, area);
        return;
    }

    let lower_h = (area.height * 2 / 5).clamp(8, 14);
    let [header, roll, lower, msg, hints] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(6),
        Constraint::Length(lower_h),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).areas(lower);

    draw_header(f, app, header);
    draw_roll(f, app, roll);
    match app.lower {
        Lower::Tracker => draw_tracker(f, app, left),
        Lower::Why => draw_why(f, app, left),
    }
    if app.is_recording() {
        draw_pitch_meter(f, app, right);
    } else {
        draw_mixer(f, app, right);
    }
    draw_message(f, app, msg);
    draw_hints(f, app, hints);
    app.areas.lower = left;
    app.areas.mixer = right;

    match app.mode {
        Mode::Picker => draw_picker(f, app, area),
        Mode::Help => draw_help(f, app, area),
        _ => {}
    }

    apply_effects(f, app, header, dt);
}

fn apply_effects(f: &mut Frame, app: &mut App, header: Rect, dt: Duration) {
    if !app.animations {
        app.fx_requests.clear();
        return;
    }
    let ease = Interpolation::QuadOut;
    for req in std::mem::take(&mut app.fx_requests) {
        match req {
            FxRequest::NotesIn => app.effects.add_unique_effect(
                FxKey::Roll,
                fx::parallel(&[
                    fx::sweep_in(Motion::LeftToRight, 24, 0, ROW_OUT, (650, ease)),
                    fx::fade_from_fg(BG, (800, ease)),
                ])
                .with_area(app.areas.roll_grid),
            ),
            FxRequest::Lower => app.effects.add_unique_effect(
                FxKey::Lower,
                fx::sweep_in(Motion::UpToDown, 6, 0, PANEL, (280, ease)).with_area(app.areas.lower),
            ),
            FxRequest::Popup => app.effects.add_unique_effect(
                FxKey::Popup,
                fx::coalesce((260, ease)).with_area(app.areas.popup),
            ),
            FxRequest::NoteFlash(i) => {
                if let Some(rect) = note_rect(app, i) {
                    app.effects.add_unique_effect(
                        FxKey::Note,
                        fx::fade_from(Color::White, Color::White, (320, ease)).with_area(rect),
                    );
                }
            }
            FxRequest::RecordStart => app.effects.add_unique_effect(
                FxKey::Header,
                fx::fade_from(RED, RED, (600, ease)).with_area(header),
            ),
        }
    }
    let area = f.area();
    app.effects.process_effects(dt.into(), f.buffer_mut(), area);
}

// ---- header -----------------------------------------------------------------

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let sep = Span::styled(" · ", Style::default().fg(FAINT));
    let mut left = vec![
        Span::styled(
            " vme ",
            Style::default()
                .fg(BG)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            app.title(),
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
    ];
    if app.dirty {
        left.push(Span::styled(" ●", Style::default().fg(ACCENT)));
    }
    left.push(sep.clone());
    left.push(Span::styled(
        app.key.map_or("no key".into(), |k| k.to_string()),
        Style::default().fg(TEXT),
    ));
    left.push(sep.clone());
    left.push(Span::styled(
        format!("{:.0} BPM", app.tempo_bpm),
        Style::default().fg(TEXT),
    ));
    left.push(sep.clone());
    left.push(Span::styled(
        app.quantize.map_or("free".into(), |g| format!("grid {g}")),
        Style::default().fg(DIM),
    ));
    left.push(sep.clone());
    left.push(Span::styled(
        format!("♪ {}", instruments::name(app.program)),
        Style::default().fg(NOTE),
    ));
    left.push(sep);
    left.push(Span::styled(
        format!("{} notes", app.notes.len()),
        Style::default().fg(DIM),
    ));

    let status = if app.is_recording() {
        let on = (app.clock * 2.0) as i64 % 2 == 0;
        Span::styled(
            format!(
                " {} REC {} ",
                if on { "●" } else { " " },
                clock_fmt(app.record_seconds)
            ),
            Style::default()
                .fg(Color::White)
                .bg(RED)
                .add_modifier(Modifier::BOLD),
        )
    } else if let Some(b) = &app.busy {
        let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let c = frames[(app.clock * 12.0) as usize % frames.len()];
        Span::styled(format!(" {c} {b}… "), Style::default().fg(BG).bg(YELLOW))
    } else if app.playing {
        let b = app.playhead.value.max(0.0);
        Span::styled(
            format!(
                " ▶ bar {} beat {:.1} ",
                (b / 4.0).floor() as i64 + 1,
                b % 4.0 + 1.0
            ),
            Style::default()
                .fg(BG)
                .bg(GREEN)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(" ■ stopped ", Style::default().fg(DIM))
    };
    let sw = status.width() as u16;
    let [l, r] = Layout::horizontal([Constraint::Min(10), Constraint::Length(sw)]).areas(area);
    f.render_widget(Paragraph::new(Line::from(left)), l);
    f.render_widget(Paragraph::new(Line::from(status)), r);
}

fn clock_fmt(secs: f64) -> String {
    format!("{}:{:04.1}", (secs / 60.0).floor() as i64, secs % 60.0)
}

// ---- piano roll -------------------------------------------------------------

fn is_black(pitch: u8) -> bool {
    matches!(pitch % 12, 1 | 3 | 6 | 8 | 10)
}

/// Screen rect of a note in the roll, if visible.
fn note_rect(app: &App, i: usize) -> Option<Rect> {
    let n = app.notes.get(i)?.note;
    let g = app.areas.roll_grid;
    let row = i32::from(app.view.top_pitch) - i32::from(n.pitch);
    if row < 0 || row >= i32::from(g.height) {
        return None;
    }
    let x0 = ((n.start_beats - app.view.left_beat) / app.view.zoom)
        .floor()
        .max(0.0);
    let x1 = ((n.start_beats + n.duration_beats - app.view.left_beat) / app.view.zoom)
        .ceil()
        .min(f64::from(g.width));
    if x1 <= x0 {
        return None;
    }
    Some(Rect::new(
        g.x + x0 as u16,
        g.y + row as u16,
        (x1 - x0) as u16,
        1,
    ))
}

fn update_view(app: &mut App, rows: i32, cols: f64) {
    let zoom = app.view.zoom;
    let span = cols * zoom;
    let clamp_top = |t: i32| t.clamp((rows - 1).min(127), 127) as u8;

    if app.is_recording() {
        if let Some(m) = app.live_pitch.value {
            let m = m.round() as i32;
            let top = i32::from(app.view.top_pitch);
            if m > top - 2 || m < top - rows + 3 {
                app.view.top_pitch = clamp_top(m + rows / 2);
            }
        }
        let head = app.record_seconds * app.tempo_bpm / 60.0;
        if head > app.view.left_beat + span * 0.85 {
            app.view.left_beat = head - span * 0.5;
        }
        return;
    }

    if std::mem::take(&mut app.view_fit) && !app.notes.is_empty() {
        let hi = app
            .notes
            .iter()
            .map(|n| i32::from(n.note.pitch))
            .max()
            .unwrap_or(72);
        let lo = app
            .notes
            .iter()
            .map(|n| i32::from(n.note.pitch))
            .min()
            .unwrap_or(60);
        let span_rows = hi - lo + 1;
        app.view.top_pitch = clamp_top(if span_rows <= rows {
            hi + (rows - span_rows) / 2
        } else {
            hi + 1
        });
        let end = app
            .notes
            .iter()
            .map(|n| n.note.start_beats + n.note.duration_beats)
            .fold(0.0, f64::max);
        // Zoom so the whole take fits if that keeps notes readable.
        let mut z = 0.25;
        while end / z > cols && z < 1.0 {
            z *= 2.0;
        }
        app.view.zoom = z;
        app.view.left_beat = 0.0;
        return;
    }

    if app.playing {
        let ph = app.playhead.value;
        if ph < app.view.left_beat || ph > app.view.left_beat + span * 0.9 {
            app.view.left_beat = (ph - span * 0.1).max(0.0);
        }
    } else if app.view_follow {
        if let Some(n) = app.notes.get(app.selected).map(|n| n.note) {
            let top = i32::from(app.view.top_pitch);
            let p = i32::from(n.pitch);
            if p > top || p <= top - rows {
                app.view.top_pitch = clamp_top(p + rows / 2);
            }
            let end = n.start_beats + n.duration_beats;
            if n.start_beats < app.view.left_beat {
                app.view.left_beat = (n.start_beats / 4.0).floor() * 4.0;
            } else if end > app.view.left_beat + span {
                app.view.left_beat = ((end - span * 0.75) / 4.0).ceil().max(0.0) * 4.0;
            }
        }
    }
}

fn draw_roll(f: &mut Frame, app: &mut App, area: Rect) {
    let col_len = format_length(app.view.zoom);
    let title = format!("Piano roll · 1 col = {col_len}");
    let block = panel(&title, app.focus == Focus::Roll);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let gutter = 5u16;
    if inner.width <= gutter + 4 || inner.height < 2 {
        return;
    }
    let grid = Rect::new(
        inner.x + gutter,
        inner.y,
        inner.width - gutter,
        inner.height,
    );
    app.areas.roll = inner;
    app.areas.roll_grid = grid;
    let rows = i32::from(grid.height);
    update_view(app, rows, f64::from(grid.width));

    let buf = f.buffer_mut();
    let top = i32::from(app.view.top_pitch);
    let zoom = app.view.zoom;
    let left = app.view.left_beat;
    let sel_pitch = app.notes.get(app.selected).map(|n| n.note.pitch);

    for r in 0..rows {
        let p = top - r;
        if !(0..=127).contains(&p) {
            continue;
        }
        let p = p as u8;
        let y = grid.y + r as u16;
        let in_key = app.key.is_none_or(|k| k.contains(p));
        // Keyboard gutter.
        let name = app.key.map_or_else(|| note_name(p), |k| k.note_name(p));
        let (kfg, kbg) = if Some(p) == sel_pitch {
            (BG, ACCENT)
        } else if is_black(p) {
            (DIM, Color::Rgb(28, 28, 34))
        } else {
            (Color::Rgb(40, 40, 52), Color::Rgb(196, 196, 206))
        };
        let label = if p % 12 == 0 || Some(p) == sel_pitch || in_key && app.key.is_some() {
            format!("{name:<4}")
        } else {
            "    ".to_string()
        };
        buf.set_string(inner.x, y, &label, Style::default().fg(kfg).bg(kbg));
        buf.set_string(inner.x + 4, y, "▏", Style::default().fg(FAINT).bg(PANEL));

        let row_bg = if in_key { ROW_IN } else { ROW_OUT };
        for c in 0..grid.width {
            let b0 = left + f64::from(c) * zoom;
            let b1 = b0 + zoom;
            let bar = (b0 / 4.0).floor() != ((b1 - 1e-9) / 4.0).floor() || (b0 % 4.0).abs() < 1e-9;
            let beat = (b0).floor() != (b1 - 1e-9).floor() || (b0 % 1.0).abs() < 1e-9;
            let bg = if bar && (b0 % 4.0) < zoom {
                BAR
            } else if beat && (b0 % 1.0) < zoom && zoom < 1.0 {
                BEAT
            } else {
                row_bg
            };
            if let Some(cell) = buf.cell_mut((grid.x + c, y)) {
                cell.set_symbol(" ").set_bg(bg).set_fg(TEXT);
            }
        }
    }

    // Notes.
    for (i, en) in app.notes.iter().enumerate() {
        let n = en.note;
        let row = top - i32::from(n.pitch);
        if row < 0 || row >= rows {
            continue;
        }
        let y = grid.y + row as u16;
        let x0 = ((n.start_beats - left) / zoom).floor() as i64;
        let x1 = ((n.start_beats + n.duration_beats - left) / zoom).ceil() as i64 - 1;
        let base = if i == app.selected { NOTE_SEL } else { NOTE };
        let color = scale(base, 0.55 + 0.45 * f32::from(n.velocity) / 127.0);
        let name = app
            .key
            .map_or_else(|| note_name(n.pitch), |k| k.note_name(n.pitch));
        for x in x0.max(0)..=x1.min(i64::from(grid.width) - 1) {
            let Some(cell) = buf.cell_mut((grid.x + x as u16, y)) else {
                continue;
            };
            if x == x1 && x1 > x0 {
                // Thin right edge so repeated notes read as separate.
                let row_bg = cell.bg;
                cell.set_symbol("▊").set_fg(color).set_bg(row_bg);
            } else {
                let ch = name
                    .chars()
                    .nth((x - x0) as usize)
                    .map_or(" ".to_string(), |c| c.to_string());
                cell.set_symbol(&ch).set_bg(color).set_fg(BG);
            }
        }
    }

    // Live contour while recording.
    if app.is_recording() {
        for &(t, m) in &app.contour {
            let Some(m) = m else { continue };
            let beat = t * app.tempo_bpm / 60.0;
            let x = ((beat - left) / zoom).floor();
            let row = top - m.round() as i32;
            if x >= 0.0 && x < f64::from(grid.width) && (0..rows).contains(&row) {
                if let Some(cell) = buf.cell_mut((grid.x + x as u16, grid.y + row as u16)) {
                    cell.set_symbol("•").set_fg(RED);
                }
            }
        }
    }

    // Playhead.
    if app.playing {
        let x = ((app.playhead.value - left) / zoom).floor();
        if x >= 0.0 && x < f64::from(grid.width) {
            for r in 0..grid.height {
                if let Some(cell) = buf.cell_mut((grid.x + x as u16, grid.y + r)) {
                    cell.set_symbol("│").set_fg(Color::White);
                }
            }
        }
    }

    if app.notes.is_empty() && !app.is_recording() {
        let msg = if app.busy.is_some() {
            "listening to your take…"
        } else {
            "press  r  and hum a melody   ·   or  :open take.wav"
        };
        let y = grid.y + grid.height / 2;
        let x = grid.x + grid.width.saturating_sub(msg.chars().count() as u16) / 2;
        buf.set_string(x, y, msg, Style::default().fg(DIM));
    }
}

// ---- tracker / why ----------------------------------------------------------

fn draw_tracker(f: &mut Frame, app: &App, area: Rect) {
    let block = panel("Tracker", app.focus == Focus::Tracker);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines = vec![Line::styled(
        "  #   bar:beat  note  len    vel  sung",
        Style::default().fg(DIM).add_modifier(Modifier::BOLD),
    )];
    let visible = inner.height.saturating_sub(1) as usize;
    let offset = app
        .selected
        .saturating_sub(visible / 2)
        .min(app.notes.len().saturating_sub(visible));
    for (i, en) in app.notes.iter().enumerate().skip(offset).take(visible) {
        let n = en.note;
        let bar = (n.start_beats / 4.0).floor() as i64 + 1;
        let beat = n.start_beats.rem_euclid(4.0) + 1.0;
        let name = app
            .key
            .map_or_else(|| note_name(n.pitch), |k| k.note_name(n.pitch));
        let mut spans = vec![Span::raw(format!(
            "{:>3}   {bar:>3}:{beat:<5.2} {name:<5} {:<6} {:>3}  ",
            i + 1,
            format_length(n.duration_beats),
            n.velocity
        ))];
        if let Some(s) = en.sung {
            let nearest = s.nearest_pitch();
            let sung_name = app
                .key
                .map_or_else(|| note_name(nearest), |k| k.note_name(nearest));
            spans.push(Span::styled(
                format!(
                    "{sung_name} {}",
                    format_cents(s.sung_pitch - f32::from(nearest))
                ),
                Style::default().fg(DIM),
            ));
            if nearest != n.pitch {
                spans.push(Span::styled(" → moved", Style::default().fg(ACCENT)));
            }
        }
        let mut line = Line::from(spans);
        if i == app.selected {
            let bg = if app.focus == Focus::Tracker {
                Color::Rgb(70, 56, 30)
            } else {
                Color::Rgb(44, 44, 60)
            };
            line = line.style(Style::default().bg(bg).fg(Color::White));
        }
        lines.push(line);
    }
    if app.notes.is_empty() {
        lines.push(Line::styled("  no notes yet", Style::default().fg(DIM)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_why(f: &mut Frame, app: &App, area: Rect) {
    let block = panel("Why notes changed", app.focus == Focus::Tracker);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(t) = &app.analysis else {
        f.render_widget(
            Paragraph::new("No transcription yet. Record a take with r, then come back here (t).")
                .style(Style::default().fg(DIM))
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    };
    let cents = (t.tuning_offset * 100.0).round();
    let mut lines = vec![Line::styled(
        if cents.abs() >= 3.0 {
            format!(
                "You sang {}¢ {} overall; corrected before choosing notes.",
                cents.abs(),
                if cents < 0.0 { "flat" } else { "sharp" }
            )
        } else {
            "You sang in tune overall.".to_string()
        },
        Style::default().fg(DIM),
    )];
    if t.changes.is_empty() {
        lines.push(Line::raw(
            "Nothing changed: every note was in key and on the grid.",
        ));
    }
    for c in &t.changes {
        let (label, color) = match c.kind {
            ChangeKind::Pitch => ("pitch ", ACCENT),
            ChangeKind::Timing => ("timing", NOTE),
            ChangeKind::Length => ("length", NOTE),
            ChangeKind::Merged => ("merged", DIM),
            ChangeKind::Dropped => ("drop  ", DIM),
        };
        let id = c
            .note
            .map_or(" - ".to_string(), |i| format!("#{:<2}", i + 1));
        let selected = c.note == Some(app.selected);
        let mut line = Line::from(vec![
            Span::styled(format!("{id} "), Style::default().fg(DIM)),
            Span::styled(format!("{label} "), Style::default().fg(color)),
            Span::raw(c.message.clone()),
        ]);
        if selected {
            line = line.style(Style::default().bg(Color::Rgb(44, 44, 60)));
        }
        lines.push(line);
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

// ---- mixer and meters -------------------------------------------------------------

fn meter_color(frac_from_bottom: f32) -> Color {
    if frac_from_bottom > 0.9 {
        RED
    } else if frac_from_bottom > 0.75 {
        YELLOW
    } else {
        GREEN
    }
}

fn draw_vmeter(buf: &mut Buffer, x: u16, y: u16, width: u16, height: u16, m: &Meter) {
    const EIGHTHS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    let h = f32::from(height);
    let level = m.fraction() * h;
    let peak_row = ((m.peak_fraction() * h).ceil() as u16).min(height);
    for r in 0..height {
        let from_bottom = f32::from(r);
        let yy = y + height - 1 - r;
        let fill = ((level - from_bottom) * 8.0).clamp(0.0, 8.0) as usize;
        let color = meter_color((from_bottom + 0.5) / h);
        for dx in 0..width {
            if let Some(cell) = buf.cell_mut((x + dx, yy)) {
                let sym = if fill == 0 && peak_row == r + 1 && m.peak_db > Meter::FLOOR {
                    "▔"
                } else {
                    EIGHTHS[fill]
                };
                cell.set_symbol(sym)
                    .set_fg(if fill == 0 { TEXT } else { color })
                    .set_bg(Color::Rgb(26, 26, 34));
            }
        }
    }
}

fn draw_mixer(f: &mut Frame, app: &App, area: Rect) {
    let block = panel("Mixer", app.focus == Focus::Mixer);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 5 {
        return;
    }
    let strip_w = 11u16;
    let meter_h = inner.height - 3;
    let buf = f.buffer_mut();
    let mut x = inner.x + 1;
    let any_solo = app.strips.iter().any(|s| s.solo);
    for (i, s) in app.strips.iter().enumerate() {
        if x + strip_w > inner.right() {
            break;
        }
        let selected = app.focus == Focus::Mixer && i == app.strip_sel;
        let name_style = if selected {
            Style::default()
                .fg(BG)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD)
        };
        buf.set_string(x, inner.y, format!("{:^9}", s.name), name_style);
        draw_vmeter(buf, x + 3, inner.y + 1, 3, meter_h, &s.meter);
        buf.set_string(
            x,
            inner.y + 1 + meter_h,
            format!("{:>6.1} dB", s.gain_db),
            Style::default().fg(DIM),
        );
        let pan = if s.pan.abs() < 0.05 {
            " C ".to_string()
        } else if s.pan < 0.0 {
            format!("L{:<2}", (s.pan.abs() * 100.0).round() as i32 / 10)
        } else {
            format!("R{:<2}", (s.pan * 100.0).round() as i32 / 10)
        };
        let silenced = s.muted || (any_solo && !s.solo);
        buf.set_string(x, inner.y + 2 + meter_h, pan, Style::default().fg(DIM));
        buf.set_string(
            x + 4,
            inner.y + 2 + meter_h,
            "M",
            Style::default()
                .fg(if s.muted { BG } else { FAINT })
                .bg(if s.muted { RED } else { PANEL }),
        );
        buf.set_string(
            x + 6,
            inner.y + 2 + meter_h,
            "S",
            Style::default()
                .fg(if s.solo { BG } else { FAINT })
                .bg(if s.solo { YELLOW } else { PANEL }),
        );
        if silenced {
            buf.set_string(
                x + 8,
                inner.y + 2 + meter_h,
                "∅",
                Style::default().fg(FAINT),
            );
        }
        x += strip_w;
    }
    if x + strip_w <= inner.right() {
        buf.set_string(
            x,
            inner.y,
            format!("{:^9}", "master"),
            Style::default().fg(DIM).add_modifier(Modifier::BOLD),
        );
        draw_vmeter(buf, x + 3, inner.y + 1, 3, meter_h, &app.master);
    }
}

fn draw_pitch_meter(f: &mut Frame, app: &App, area: Rect) {
    let block = panel("Pitch meter", true).border_style(Style::default().fg(RED));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines: Vec<Line> = Vec::new();
    match (app.live_pitch.value, app.live_hz) {
        (Some(m), hz) => {
            let nearest = m.round().clamp(0.0, 127.0) as u8;
            let cents = m - f32::from(nearest);
            let name = app
                .key
                .map_or_else(|| note_name(nearest), |k| k.note_name(nearest));
            let in_key = app.key.map(|k| k.contains(nearest));
            let color = if cents.abs() < 0.12 {
                GREEN
            } else if cents.abs() < 0.3 {
                YELLOW
            } else {
                RED
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {name:^5}  "),
                    Style::default()
                        .fg(BG)
                        .bg(color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}  {:.0} Hz", format_cents(cents), hz.unwrap_or(0.0)),
                    Style::default().fg(TEXT),
                ),
            ]));
            let w = inner.width.saturating_sub(4).clamp(11, 41) as i32 | 1;
            let half = w / 2;
            let pos = (half + (cents * 2.0 * half as f32).round() as i32).clamp(0, w - 1);
            let gauge: String = (0..w)
                .map(|i| {
                    if i == pos {
                        '●'
                    } else if i == half {
                        '┼'
                    } else {
                        '─'
                    }
                })
                .collect();
            lines.push(Line::from(vec![
                Span::styled(" ♭ ", Style::default().fg(DIM)),
                Span::styled(gauge, Style::default().fg(color)),
                Span::styled(" ♯", Style::default().fg(DIM)),
            ]));
            if let Some(ik) = in_key {
                lines.push(Line::styled(
                    if ik {
                        "  in key".to_string()
                    } else {
                        "  outside the key (will be snapped)".to_string()
                    },
                    Style::default().fg(if ik { GREEN } else { ACCENT }),
                ));
            }
        }
        _ => {
            lines.push(Line::styled("  …hum something", Style::default().fg(DIM)));
            lines.push(Line::raw(""));
        }
    }
    lines.push(Line::raw(""));
    f.render_widget(Paragraph::new(lines), inner);
    // Horizontal mic level meter on the last row.
    let y = inner.bottom().saturating_sub(1);
    let w = inner.width.saturating_sub(8);
    let buf = f.buffer_mut();
    buf.set_string(inner.x + 1, y, "mic ", Style::default().fg(DIM));
    let filled = (app.mic.fraction() * f32::from(w)) as u16;
    let peak = ((app.mic.peak_fraction() * f32::from(w)) as u16).min(w.saturating_sub(1));
    for i in 0..w {
        let frac = f32::from(i) / f32::from(w.max(1));
        let (sym, fg) = if i < filled {
            ("█", meter_color(frac))
        } else if i == peak && app.mic.peak_db > Meter::FLOOR {
            ("▏", TEXT)
        } else {
            ("·", FAINT)
        };
        buf.set_string(inner.x + 5 + i, y, sym, Style::default().fg(fg));
    }
}

// ---- bottom lines -----------------------------------------------------------

fn draw_message(f: &mut Frame, app: &App, area: Rect) {
    if app.mode == Mode::Command {
        let word = app.command.split_whitespace().next().unwrap_or("");
        let suggestions: Vec<&str> = COMMANDS
            .iter()
            .filter(|(n, _)| !word.is_empty() && n.starts_with(word) && app.command.trim() == word)
            .map(|(_, usage)| *usage)
            .take(1)
            .collect();
        let mut spans = vec![
            Span::styled(
                ":",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(app.command.clone()),
            Span::styled("█", Style::default().fg(ACCENT)),
        ];
        if let Some(s) = suggestions.first() {
            spans.push(Span::styled(format!("    {s}"), Style::default().fg(FAINT)));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }
    if let Some((msg, level)) = &app.message {
        let (icon, color) = match level {
            Level::Info => ("›", TEXT),
            Level::Warn => ("!", ACCENT),
            Level::Error => ("✗", RED),
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" {icon} "),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(msg.clone(), Style::default().fg(color)),
            ])),
            area,
        );
    }
}

fn hints_for(app: &App) -> Vec<(&'static str, &'static str)> {
    match app.mode {
        Mode::Recording => vec![("r/␣/⏎", "stop & transcribe"), ("esc", "cancel")],
        Mode::Command => vec![
            ("⏎", "run"),
            ("esc", "cancel"),
            ("↑/↓", "history"),
            (":help", "all commands"),
        ],
        Mode::Picker => vec![
            ("type", "search"),
            ("↑/↓", "select"),
            ("⏎", "choose"),
            ("esc", "cancel"),
        ],
        Mode::Help => vec![("esc", "close")],
        Mode::Normal => {
            let mut h = vec![("r", "record"), ("␣", "play"), ("⏎", "play from note")];
            if app.focus == Focus::Mixer {
                h.extend([
                    ("h/l", "strip"),
                    ("j/k", "volume"),
                    ("[ ]", "pan"),
                    ("m", "mute"),
                    ("s", "solo"),
                ]);
            } else {
                h.extend([
                    ("h/l", "note"),
                    ("j/k", "pitch"),
                    ("J/K", "octave"),
                    ("< >", "length"),
                    ("H/L", "move"),
                    ("x", "delete"),
                    ("u", "undo"),
                    ("+/-", "zoom"),
                ]);
            }
            h.extend([
                ("i", "instrument"),
                ("t", "why"),
                ("tab", "panel"),
                (":", "command"),
                ("?", "help"),
                ("q", "quit"),
            ]);
            h
        }
    }
}

fn draw_hints(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    let mut used = 0usize;
    for (k, label) in hints_for(app) {
        let w = k.chars().count() + label.chars().count() + 3;
        if used + w > usize::from(area.width) {
            break;
        }
        used += w;
        spans.push(Span::styled(
            format!(" {k}"),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {label} "), Style::default().fg(DIM)));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(24, 24, 32))),
        area,
    );
}

// ---- popups -------------------------------------------------------------------

fn centered(area: Rect, w_pct: u16, h_pct: u16, min_w: u16, min_h: u16) -> Rect {
    let w = (area.width * w_pct / 100).max(min_w).min(area.width);
    let h = (area.height * h_pct / 100).max(min_h).min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn draw_picker(f: &mut Frame, app: &mut App, area: Rect) {
    let rect = centered(area, 46, 66, 40, 12);
    app.areas.popup = rect;
    f.render_widget(Clear, rect);
    let block = panel("Instrument", true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                " › ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(app.picker_query.clone()),
            Span::styled("█", Style::default().fg(ACCENT)),
        ]),
        Line::raw(""),
    ];
    let visible = inner.height.saturating_sub(2) as usize;
    let offset = app.picker_sel.saturating_sub(visible.saturating_sub(1));
    for (i, (p, _)) in app
        .picker_results
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible)
    {
        let current = *p == app.program;
        let style = if i == app.picker_sel {
            Style::default()
                .fg(BG)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TEXT)
        };
        lines.push(Line::styled(
            format!(
                " {:>3}  {:<28} {}",
                p + 1,
                instruments::name(*p),
                if current { "●" } else { "" }
            ),
            style,
        ));
    }
    if app.picker_results.is_empty() {
        lines.push(Line::styled("   no match", Style::default().fg(DIM)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_help(f: &mut Frame, app: &mut App, area: Rect) {
    let rect = centered(area, 80, 84, 60, 16);
    app.areas.popup = rect;
    f.render_widget(Clear, rect);
    let block = panel("Help", true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let [l, r] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(inner);
    let key = |k: &str, d: &str| {
        Line::from(vec![
            Span::styled(
                format!(" {k:<9}"),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(d.to_string(), Style::default().fg(TEXT)),
        ])
    };
    let head = |t: &str| {
        Line::styled(
            format!(" {t}"),
            Style::default().fg(DIM).add_modifier(Modifier::BOLD),
        )
    };
    let keys = vec![
        head("Everywhere"),
        key("r", "record a take (again to stop)"),
        key("space", "play / stop"),
        key("enter", "play from the selected note"),
        key("i", "instrument picker"),
        key("t", "tracker ⇄ why notes changed"),
        key("tab", "next panel"),
        key(":", "command line"),
        key("q", "quit"),
        Line::raw(""),
        head("Notes (roll / tracker)"),
        key("h l", "previous / next note"),
        key("j k", "pitch down / up"),
        key("J K", "octave down / up"),
        key("< >", "shorter / longer (grid)"),
        key("H L", "move earlier / later"),
        key("x", "delete"),
        key("u", "undo"),
        key("+ -", "zoom in / out"),
        Line::raw(""),
        head("Mixer"),
        key("h l", "select strip"),
        key("j k", "volume"),
        key("[ ]", "pan"),
        key("m s", "mute / solo"),
    ];
    let mut cmds = vec![head("Commands")];
    cmds.extend(
        COMMANDS
            .iter()
            .map(|(_, usage)| Line::styled(format!(" {usage}"), Style::default().fg(TEXT))),
    );
    cmds.push(Line::raw(""));
    cmds.push(head("Tips"));
    for tip in [
        "Hum 'doo' or 'la' so repeated notes stay separate.",
        "Rows shaded lighter are in the key; out-of-key",
        "notes you sing are snapped to the nearest one.",
        "Takes are saved in ./takes; :w writes the MIDI.",
    ] {
        cmds.push(Line::styled(format!(" {tip}"), Style::default().fg(DIM)));
    }
    f.render_widget(Paragraph::new(keys), l);
    f.render_widget(Paragraph::new(cmds).wrap(Wrap { trim: false }), r);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{EditNote, Options};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use vme_core::sequence::SeqNote;

    fn app() -> App {
        let mut app = App::new(&Options {
            file: None,
            animations: true,
            mouse: false,
        });
        app.notes = [(60u8, 0.0), (64, 1.0), (67, 2.0), (72, 3.0)]
            .iter()
            .map(|&(p, s)| EditNote {
                note: SeqNote {
                    pitch: p,
                    start_beats: s,
                    duration_beats: 1.0,
                    velocity: 100,
                },
                sung: None,
            })
            .collect();
        app.key = Some("C major".parse().unwrap());
        app.view_fit = true;
        app
    }

    fn render(app: &mut App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app, Duration::from_millis(16)))
            .unwrap();
        let buf = term.backend().buffer().clone();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn renders_main_screen_at_many_sizes() {
        for (w, h) in [(64, 18), (80, 24), (120, 40), (200, 60)] {
            let mut a = app();
            let screen = render(&mut a, w, h);
            assert!(screen.contains("vme"), "{w}x{h}");
            assert!(screen.contains("C major"), "{w}x{h}");
            assert!(screen.contains("Piano roll"), "{w}x{h}");
            assert!(screen.contains("Mixer"), "{w}x{h}");
        }
    }

    #[test]
    fn too_small_terminal_says_so() {
        let mut a = app();
        assert!(render(&mut a, 40, 10).contains("at least"));
    }

    #[test]
    fn notes_show_in_roll_and_tracker() {
        let mut a = app();
        let screen = render(&mut a, 120, 40);
        assert!(screen.contains("E4"));
        assert!(screen.contains("1/4"));
        // Fitting puts the highest note near the top.
        assert!(a.view.top_pitch >= 72);
    }

    #[test]
    fn popups_and_modes_render() {
        let mut a = app();
        a.open_picker();
        assert!(render(&mut a, 100, 30).contains("Violin"));
        a.mode = Mode::Help;
        assert!(render(&mut a, 100, 30).contains("Commands"));
        a.mode = Mode::Command;
        a.command = "tempo".into();
        assert!(render(&mut a, 100, 30).contains(":tempo 100"));
        a.mode = Mode::Normal;
        a.toggle_lower();
        assert!(render(&mut a, 100, 30).contains("Why notes changed"));
    }

    #[test]
    fn animations_off_skips_effects() {
        let mut a = app();
        a.animations = false;
        a.fx_requests.push(FxRequest::NotesIn);
        render(&mut a, 100, 30);
        assert!(!a.effects.is_running());
    }
}
