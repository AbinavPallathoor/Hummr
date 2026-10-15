//! Drawing. Every frame is drawn from `App` state; effects are applied last.

use std::time::Duration;

use hummr_core::instruments;
use hummr_core::notation::format_length;
use hummr_core::theory::{format_cents, note_name};
use hummr_core::transcribe::ChangeKind;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use tachyonfx::{Interpolation, Motion, fx};

use crate::app::{App, Focus, FxKey, FxRequest, Level, Lower, Mode, Strip};
use crate::command::COMMANDS;
use crate::smooth::Meter;
use crate::song::Placement;

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
            "hummr needs at least 64×18 characters (now {}×{}).\nMake the terminal bigger, or press q to quit.",
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
            " hummr ",
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
    if let Some(t) = app.track() {
        left.push(Span::styled(
            format!(
                "♪ {}/{} {}",
                app.cur + 1,
                app.tracks.len(),
                t.display_name()
            ),
            Style::default()
                .fg(track_color(app.cur))
                .add_modifier(Modifier::BOLD),
        ));
        left.push(sep);
    }
    left.push(Span::styled(
        format!("{} notes", app.note_count()),
        Style::default().fg(DIM),
    ));

    let status = if let Some(n) = app.count_in_left {
        Span::styled(
            format!(" count-in {n} "),
            Style::default()
                .fg(BG)
                .bg(YELLOW)
                .add_modifier(Modifier::BOLD),
        )
    } else if app.is_recording() {
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
        let b = app.cursor;
        Span::styled(
            format!(
                " ■ bar {} beat {} ",
                (b / 4.0).floor() as i64 + 1,
                trim_beat(b % 4.0 + 1.0)
            ),
            Style::default().fg(DIM),
        )
    };
    let sw = status.width() as u16;
    let [l, r] = Layout::horizontal([Constraint::Min(10), Constraint::Length(sw)]).areas(area);
    f.render_widget(Paragraph::new(Line::from(left)), l);
    f.render_widget(Paragraph::new(Line::from(status)), r);
}

fn trim_beat(b: f64) -> String {
    let r = (b * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-9 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

fn clock_fmt(secs: f64) -> String {
    format!("{}:{:04.1}", (secs / 60.0).floor() as i64, secs % 60.0)
}

// ---- piano roll -------------------------------------------------------------

/// Colour of each track's notes, so layered parts stay tellable apart.
pub fn track_color(i: usize) -> Color {
    const PALETTE: [Color; 6] = [
        NOTE,
        Color::Rgb(214, 120, 230),
        Color::Rgb(120, 214, 120),
        Color::Rgb(240, 140, 90),
        Color::Rgb(120, 150, 250),
        Color::Rgb(230, 210, 100),
    ];
    PALETTE[i % PALETTE.len()]
}

fn is_black(pitch: u8) -> bool {
    matches!(pitch % 12, 1 | 3 | 6 | 8 | 10)
}

/// Roll row (0 = top) that shows `pitch`.
fn row_of(app: &App, pitch: u8) -> i32 {
    (i32::from(app.view.top_pitch) - i32::from(pitch))
        .div_euclid(i32::from(app.view.pitch_step.max(1)))
}

/// Screen rect of a note in the roll, if visible.
fn note_rect(app: &App, i: usize) -> Option<Rect> {
    let n = app.notes().get(i)?.note;
    let g = app.areas.roll_grid;
    let row = row_of(app, n.pitch);
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
    let step = i32::from(app.view.pitch_step.max(1));
    let clamp_top = |t: i32| t.clamp((rows * step - 1).min(127), 127) as u8;

    if app.is_recording() {
        if let Some(m) = app.live_pitch.value {
            let m = m.round() as i32;
            let top = i32::from(app.view.top_pitch);
            if m > top - 2 || m < top - rows * step + 3 {
                app.view.top_pitch = clamp_top(m + rows * step / 2);
            }
        }
        if let Some(head) = app.record_beat {
            if head > app.view.left_beat + span * 0.85 || head < app.view.left_beat {
                app.view.left_beat = (head - span * 0.25).max(0.0);
            }
        }
        return;
    }

    if std::mem::take(&mut app.view_fit) && app.note_count() > 0 {
        let all = || app.tracks.iter().flat_map(|t| t.notes.iter());
        let hi = all().map(|n| i32::from(n.note.pitch)).max().unwrap_or(72);
        let lo = all().map(|n| i32::from(n.note.pitch)).min().unwrap_or(60);
        // Squeeze several semitones into a row if that is what it takes.
        let span_rows = hi - lo + 1;
        let fit_step = ((span_rows + rows - 1) / rows.max(1)).clamp(1, 12);
        app.view.pitch_step = fit_step as u8;
        let visible = rows * fit_step;
        let top = if span_rows <= visible {
            hi + (visible - span_rows) / 2
        } else {
            hi
        };
        app.view.top_pitch = top.clamp((visible - 1).min(127), 127) as u8;
        let end = app.tracks.iter().map(|t| t.end_beats()).fold(0.0, f64::max);
        // The most detailed zoom at which every note of every track fits.
        let mut z = 0.25;
        while (end + z) / z > cols && z < 2.0 {
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
        // Keep the playhead on screen while scrubbing.
        let c = app.cursor;
        if c < app.view.left_beat || c >= app.view.left_beat + span {
            app.view.left_beat = ((c - span * 0.25) / 4.0).floor().max(0.0) * 4.0;
        }
        if let Some(n) = app.notes().get(app.selected).map(|n| n.note) {
            let top = i32::from(app.view.top_pitch);
            let p = i32::from(n.pitch);
            if p > top || p <= top - rows * step {
                app.view.top_pitch = clamp_top(p + rows * step / 2);
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
    let inner = panel("", false).inner(area);
    let gutter = 5u16;
    if inner.width <= gutter + 4 || inner.height < 3 {
        return;
    }
    // Row 0: track tabs, row 1: bar ruler, then the note grid.
    let tabs_row = Rect::new(inner.x, inner.y, inner.width, 1);
    let ruler = Rect::new(inner.x + gutter, inner.y + 1, inner.width - gutter, 1);
    let grid = Rect::new(
        inner.x + gutter,
        inner.y + 2,
        inner.width - gutter,
        inner.height - 2,
    );
    app.areas.roll = inner;
    app.areas.roll_grid = grid;
    app.areas.ruler = ruler;
    let rows = i32::from(grid.height);
    update_view(app, rows, f64::from(grid.width));
    let mut title = format!("Piano roll · 1 col = {}", format_length(app.view.zoom));
    if app.view.pitch_step > 1 {
        title.push_str(&format!(
            " · {} semitones per row (+ to zoom in)",
            app.view.pitch_step
        ));
    }
    f.render_widget(panel(&title, app.focus == Focus::Roll), area);
    let step = i32::from(app.view.pitch_step.max(1));

    let buf = f.buffer_mut();
    let top = i32::from(app.view.top_pitch);
    let zoom = app.view.zoom;
    let left = app.view.left_beat;
    let sel_pitch = app.notes().get(app.selected).map(|n| n.note.pitch);
    let head = app.record_beat.unwrap_or(if app.playing {
        app.playhead.value
    } else {
        app.cursor
    });

    // Track tabs: one per instrument, numbered like the 1-9 keys.
    app.areas.tabs = [None; 16];
    let mut x = tabs_row.x;
    buf.set_string(x, tabs_row.y, "trk ", Style::default().fg(FAINT));
    x += 4;
    for (i, t) in app.tracks.iter().enumerate() {
        let label = format!(" {} {} ", i + 1, t.display_name());
        let w = label.chars().count() as u16;
        if x + w + 1 > tabs_row.right() {
            buf.set_string(x, tabs_row.y, "…", Style::default().fg(DIM));
            break;
        }
        let style = if i == app.cur {
            Style::default()
                .fg(BG)
                .bg(track_color(i))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(scale(track_color(i), 0.6))
                .bg(Color::Rgb(28, 28, 38))
        };
        buf.set_string(x, tabs_row.y, &label, style);
        if let Some(slot) = app.areas.tabs.get_mut(i) {
            *slot = Some((Rect::new(x, tabs_row.y, w, 1), i));
        }
        x += w + 1;
    }
    if app.tracks.is_empty() {
        buf.set_string(x, tabs_row.y, "no tracks yet", Style::default().fg(FAINT));
    }

    // Ruler: bar numbers, beat ticks and the playhead marker.
    buf.set_string(
        inner.x,
        ruler.y,
        "bar ▏",
        Style::default().fg(FAINT).bg(PANEL),
    );
    for c in 0..ruler.width {
        if let Some(cell) = buf.cell_mut((ruler.x + c, ruler.y)) {
            cell.set_symbol(" ").set_bg(Color::Rgb(24, 24, 33));
        }
    }
    let mut c = 0u16;
    while c < ruler.width {
        let b0 = left + f64::from(c) * zoom;
        let starts_bar = (b0 % 4.0) < zoom - 1e-9 || (b0 % 4.0).abs() < 1e-9;
        let starts_beat = (b0 % 1.0) < zoom - 1e-9 || (b0 % 1.0).abs() < 1e-9;
        if starts_bar {
            let label = format!("{}", (b0 / 4.0).floor() as i64 + 1);
            buf.set_string(
                ruler.x + c,
                ruler.y,
                &label,
                Style::default().fg(DIM).bg(Color::Rgb(24, 24, 33)),
            );
            c += label.len() as u16;
            continue;
        } else if starts_beat && zoom < 1.0 {
            buf.set_string(
                ruler.x + c,
                ruler.y,
                "·",
                Style::default().fg(FAINT).bg(Color::Rgb(24, 24, 33)),
            );
        }
        c += 1;
    }
    let head_x = ((head - left) / zoom).floor();
    let head_color = if app.is_recording() {
        RED
    } else if app.playing {
        Color::White
    } else {
        ACCENT
    };
    if head_x >= 0.0 && head_x < f64::from(ruler.width) {
        buf.set_string(
            ruler.x + head_x as u16,
            ruler.y,
            "▼",
            Style::default()
                .fg(head_color)
                .bg(Color::Rgb(24, 24, 33))
                .add_modifier(Modifier::BOLD),
        );
    }

    for r in 0..rows {
        let p = top - r * step;
        if !(0..=127).contains(&p) {
            continue;
        }
        let p = p as u8;
        // When a row holds several semitones, label it by the C inside it.
        let row_pitches = || {
            (0..step)
                .map(move |k| i32::from(p) - k)
                .filter(|q| *q >= 0)
                .map(|q| q as u8)
        };
        let has_sel = sel_pitch.is_some_and(|s| row_pitches().any(|q| q == s));
        let y = grid.y + r as u16;
        let in_key = step > 1 || app.key.is_none_or(|k| k.contains(p));
        // Keyboard gutter.
        let shown = if step == 1 {
            Some(p)
        } else {
            row_pitches().find(|q| q % 12 == 0)
        };
        let name = shown.map_or_else(String::new, |q| {
            app.key.map_or_else(|| note_name(q), |k| k.note_name(q))
        });
        let (kfg, kbg) = if has_sel {
            (BG, ACCENT)
        } else if is_black(p) {
            (DIM, Color::Rgb(28, 28, 34))
        } else {
            (Color::Rgb(40, 40, 52), Color::Rgb(196, 196, 206))
        };
        let label = if step > 1 || p % 12 == 0 || has_sel || in_key && app.key.is_some() {
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

    // Other tracks first, dimmed, then the track being edited on top.
    for (ti, track) in app.tracks.iter().enumerate() {
        if ti == app.cur {
            continue;
        }
        // Dulled: the same blocks as the current track, much darker.
        let color = scale(track_color(ti), 0.32);
        for en in &track.notes {
            let n = en.note;
            let row = row_of(app, n.pitch);
            if row < 0 || row >= rows {
                continue;
            }
            let x0 = ((n.start_beats - left) / zoom).floor() as i64;
            let x1 = ((n.start_beats + n.duration_beats - left) / zoom).ceil() as i64 - 1;
            for x in x0.max(0)..=x1.min(i64::from(grid.width) - 1) {
                if let Some(cell) = buf.cell_mut((grid.x + x as u16, grid.y + row as u16)) {
                    if x == x1 && x1 > x0 {
                        let row_bg = cell.bg;
                        cell.set_symbol("▊").set_fg(color).set_bg(row_bg);
                    } else {
                        cell.set_symbol(" ").set_bg(color);
                    }
                }
            }
        }
    }

    // Notes of the current track.
    for (i, en) in app.notes().iter().enumerate() {
        let n = en.note;
        let row = row_of(app, n.pitch);
        if row < 0 || row >= rows {
            continue;
        }
        let y = grid.y + row as u16;
        let x0 = ((n.start_beats - left) / zoom).floor() as i64;
        let x1 = ((n.start_beats + n.duration_beats - left) / zoom).ceil() as i64 - 1;
        let base = if i == app.selected {
            NOTE_SEL
        } else {
            track_color(app.cur)
        };
        let mut color = scale(base, 0.55 + 0.45 * f32::from(n.velocity) / 127.0);
        let replacing = app.recording_plan().is_some_and(|p| {
            p.placement == Some(Placement::Overwrite) && n.start_beats >= p.at_beat - 1e-9
        });
        if replacing {
            // These are being recorded over.
            color = scale(color, 0.35);
        }
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
        for &(beat, m) in &app.contour {
            let Some(m) = m else { continue };
            let x = ((beat - left) / zoom).floor();
            let row = (top - m.round() as i32).div_euclid(step);
            if x >= 0.0 && x < f64::from(grid.width) && (0..rows).contains(&row) {
                if let Some(cell) = buf.cell_mut((grid.x + x as u16, grid.y + row as u16)) {
                    cell.set_symbol("•").set_fg(RED);
                }
            }
        }
    }

    // Playhead line, always visible: amber when stopped, white when
    // playing, red when recording.
    if head_x >= 0.0 && head_x < f64::from(grid.width) {
        for r in 0..grid.height {
            if let Some(cell) = buf.cell_mut((grid.x + head_x as u16, grid.y + r)) {
                cell.set_symbol(if app.playing || app.is_recording() {
                    "│"
                } else {
                    "┊"
                })
                .set_fg(head_color);
            }
        }
    }

    if app.note_count() == 0 && !app.is_recording() {
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
    let title = match app.track() {
        Some(t) => format!("Tracker · {} {}", app.cur + 1, t.display_name()),
        None => "Tracker".to_string(),
    };
    let block = panel(&title, app.focus == Focus::Tracker);
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
        .min(app.notes().len().saturating_sub(visible));
    for (i, en) in app.notes().iter().enumerate().skip(offset).take(visible) {
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
    if app.notes().is_empty() {
        lines.push(Line::styled("  no notes yet", Style::default().fg(DIM)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_why(f: &mut Frame, app: &App, area: Rect) {
    let block = panel("Why notes changed (last take)", app.focus == Focus::Tracker);
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
    // Tracks, then the recorded voice; master sits at the right edge.
    let strips: Vec<(String, Color, &Strip)> = app
        .tracks
        .iter()
        .zip(&app.strips)
        .enumerate()
        .map(|(i, (t, s))| (format!("{} {}", i + 1, t.display_name()), track_color(i), s))
        .chain(std::iter::once(("voice".to_string(), DIM, &app.voice)))
        .collect();
    let room = (inner.width.saturating_sub(1 + strip_w) / strip_w).max(1) as usize;
    let focus_i = if app.focus == Focus::Mixer {
        app.mixer_sel
    } else {
        app.cur
    };
    let first = focus_i
        .saturating_sub(room - 1)
        .min(strips.len().saturating_sub(room));
    let any_solo = strips.iter().any(|(_, _, s)| s.solo);
    let buf = f.buffer_mut();
    let mut x = inner.x + 1;
    if first > 0 {
        buf.set_string(inner.x, inner.y, "‹", Style::default().fg(DIM));
    }
    for (i, (name, color, s)) in strips.iter().enumerate().skip(first).take(room) {
        let selected = app.focus == Focus::Mixer && i == app.mixer_sel;
        let current = i == app.cur && i < app.tracks.len();
        let label: String = name.chars().take(9).collect();
        let name_style = if selected {
            Style::default()
                .fg(BG)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else if current {
            Style::default()
                .fg(*color)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(*color)
        };
        buf.set_string(x, inner.y, format!("{label:^9}"), name_style);
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
        let y = inner.y + 2 + meter_h;
        buf.set_string(x, y, pan, Style::default().fg(DIM));
        buf.set_string(
            x + 4,
            y,
            "M",
            Style::default()
                .fg(if s.muted { BG } else { FAINT })
                .bg(if s.muted { RED } else { PANEL }),
        );
        buf.set_string(
            x + 6,
            y,
            "S",
            Style::default()
                .fg(if s.solo { BG } else { FAINT })
                .bg(if s.solo { YELLOW } else { PANEL }),
        );
        if silenced {
            buf.set_string(x + 8, y, "∅", Style::default().fg(FAINT));
        }
        x += strip_w;
    }
    if first + room < strips.len() {
        buf.set_string(x.saturating_sub(1), inner.y, "›", Style::default().fg(DIM));
    }
    let mx = inner.right().saturating_sub(strip_w);
    buf.set_string(
        mx,
        inner.y,
        format!("{:^9}", "master"),
        Style::default().fg(DIM).add_modifier(Modifier::BOLD),
    );
    draw_vmeter(buf, mx + 3, inner.y + 1, 3, meter_h, &app.master);
}

fn draw_pitch_meter(f: &mut Frame, app: &App, area: Rect) {
    let block = panel("Pitch meter", true).border_style(Style::default().fg(RED));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines: Vec<Line> = Vec::new();
    if let Some(n) = app.count_in_left {
        let digit = Style::default()
            .fg(BG)
            .bg(YELLOW)
            .add_modifier(Modifier::BOLD);
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled("  get ready…  ", Style::default().fg(TEXT)),
            Span::styled(format!("  {n}  "), digit),
        ]));
        lines.push(Line::styled(
            "  start singing on the next beat 1",
            Style::default().fg(DIM),
        ));
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }
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
            let mut h = vec![
                ("r", "new layer"),
                ("R", "redo track"),
                ("␣", "play"),
                ("[ ]", "scrub"),
            ];
            if app.tracks.len() > 1 {
                h.push(("1-9", "track"));
            }
            if app.focus == Focus::Mixer {
                h.extend([
                    ("h/l", "strip"),
                    ("j/k", "volume"),
                    ("[ ]", "pan"),
                    ("m", "mute"),
                    ("s", "solo"),
                    ("x", "delete track"),
                ]);
            } else {
                h.extend([
                    ("h/l", "note"),
                    ("j/k", "pitch"),
                    ("J/K", "octave"),
                    ("< >", "length"),
                    ("H/L", "move"),
                    ("a", "add note"),
                    ("x", "delete"),
                    ("M", "join next"),
                    ("D", "delete track"),
                    ("u", "undo"),
                    ("+/-", "zoom"),
                    ("0", "show all"),
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
        let current = app.track().is_some_and(|t| t.program == *p);
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
    let rect = centered(area, 96, 94, 60, 16);
    app.areas.popup = rect;
    f.render_widget(Clear, rect);
    let block = panel("Help · esc to close", true);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let [l, m, r] = Layout::horizontal([
        Constraint::Percentage(36),
        Constraint::Percentage(27),
        Constraint::Percentage(37),
    ])
    .areas(inner);
    let key = |k: &str, d: &str| {
        Line::from(vec![
            Span::styled(
                format!(" {k:<8}"),
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
    let left = vec![
        head("Everywhere"),
        key("r", "record a new layer"),
        key("R", "re-record this track"),
        key("space", "play / stop at ▼"),
        key("[ ] { }", "scrub beat / bar"),
        key("0", "show all notes"),
        key("home", "playhead to start"),
        key("1-9", "switch track"),
        key("D", "delete track (u undoes)"),
        key("i", "instrument"),
        key("t", "tracker ⇄ why"),
        key("tab", "next panel"),
        key(":", "command line"),
        key("q", "quit"),
        Line::raw(""),
        head("Notes"),
        key("a", "add a note at ▼"),
        key("h l", "previous / next"),
        key("j k", "pitch down / up"),
        key("J K", "octave down / up"),
        key("< >", "shorter / longer"),
        key("H L", "earlier / later"),
        key("M", "join with next"),
        key("x", "delete note"),
        key("u", "undo"),
        key("enter", "play from note"),
        key("+ -", "zoom"),
    ];
    let mut middle = vec![
        head("Mixer (tab to it)"),
        key("h l", "select strip"),
        key("j k", "volume"),
        key("[ ]", "pan"),
        key("m s", "mute / solo"),
        key("x", "delete its track"),
        Line::raw(""),
        head("Tips"),
    ];
    for tip in [
        "Hum 'doo' or 'la' so",
        "repeated notes stay apart.",
        "Use headphones for layers.",
        "A voice crack split a long",
        "note? Select it, press M.",
        "Lighter rows are in the key.",
        "Takes are kept in ./takes.",
    ] {
        middle.push(Line::styled(format!(" {tip}"), Style::default().fg(DIM)));
    }
    let mut cmds = vec![head("Commands")];
    cmds.extend(
        COMMANDS
            .iter()
            .map(|(_, usage)| Line::styled(format!(" {usage}"), Style::default().fg(TEXT))),
    );
    f.render_widget(Paragraph::new(left), l);
    f.render_widget(Paragraph::new(middle), m);
    f.render_widget(Paragraph::new(cmds), r);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{EditNote, Options, Track};
    use hummr_core::sequence::SeqNote;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn app() -> App {
        let mut app = App::new(&Options {
            file: None,
            animations: true,
            mouse: false,
        });
        let notes = [(60u8, 0.0), (64, 1.0), (67, 2.0), (72, 3.0)]
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
        app.tracks = vec![Track {
            notes,
            ..Track::new(40)
        }];
        app.strips = vec![Strip::default()];
        app.key = Some("C major".parse().unwrap());
        app.view_fit = true;
        app
    }

    fn with_layer(mut a: App) -> App {
        let mut bass = a.tracks[0].clone();
        bass.program = 33;
        for n in &mut bass.notes {
            n.note.pitch -= 24;
        }
        a.tracks.push(bass);
        a.strips.push(Strip::default());
        a.view_fit = true;
        a
    }

    #[test]
    fn ruler_and_playhead_are_always_visible() {
        let mut a = app();
        a.animations = false;
        a.cursor = 4.0;
        let screen = render(&mut a, 110, 34);
        let ruler = screen.lines().nth(3).unwrap();
        assert!(ruler.contains('▼'), "{ruler}");
        assert!(ruler.contains(" 3"), "bar numbers: {ruler}");
        assert!(screen.contains("■ bar 2 beat 1"), "{screen}");
        assert!(screen.contains("new layer") && screen.contains("redo track"));
    }

    #[test]
    fn layers_show_in_header_mixer_and_roll() {
        let mut a = with_layer(app());
        a.select_track(1);
        let screen = render(&mut a, 160, 40);
        assert!(screen.contains("♪ 2/2"), "{screen}");
        assert!(screen.contains("2 Electric"), "{screen}");
        assert!(screen.contains("1 Violin"), "{screen}");
        assert!(screen.contains("voice"));
        let tabs = screen.lines().nth(2).unwrap();
        assert!(
            tabs.contains("1 Violin") && tabs.contains("2 Electric"),
            "{tabs}"
        );
        assert!(screen.contains("1-9"));
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
            assert!(screen.contains("hummr"), "{w}x{h}");
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
    fn zero_fits_every_note_of_every_track() {
        let mut a = with_layer(app()); // violin C4-C5, bass two octaves down
        a.animations = false;
        a.cursor = 40.0;
        a.view.left_beat = 32.0;
        a.view_follow = false;
        a.fit_view();
        let screen = render(&mut a, 100, 26);
        assert!(a.view.pitch_step > 1, "36 semitones need squeezing");
        assert_eq!(a.view.left_beat, 0.0);
        let grid = a.areas.roll_grid;
        for t in &a.tracks {
            for n in &t.notes {
                let r = super::row_of(&a, n.note.pitch);
                assert!(
                    (0..i32::from(grid.height)).contains(&r),
                    "pitch {} off screen",
                    n.note.pitch
                );
            }
        }
        assert!(screen.contains("semitones per row"));
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
