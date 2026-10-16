//! Drawing the timeline. Everything is drawn from `Studio` state.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Paragraph, Wrap};

use super::peaks::glyph;
use super::{HEADER_W, LANE_H, Mode, Studio, clip_label};
use crate::app::Level;
use crate::ui::{ACCENT, BG, DIM, FAINT, GREEN, RED, ROW_IN, TEXT, YELLOW, scale, track_color};

/// Smallest usable terminal.
pub const MIN_W: u16 = 80;
pub const MIN_H: u16 = 24;

/// Lines that are not track rows: borders, title, ruler, status, hints.
const CHROME_H: u16 = 10;

fn fmt_time(secs: f64) -> String {
    let secs = secs.max(0.0);
    format!("{:02}:{:04.1}", (secs / 60.0).floor() as i64, secs % 60.0)
}

fn fmt_beats(b: f64) -> String {
    if (b - b.round()).abs() < 1e-6 {
        format!("{}", b.round() as i64)
    } else {
        format!("{:.2}", b)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

/// Writes `text` at (x, y), clipped to `max_x` (exclusive).
fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, max_x: u16, style: Style) {
    for (cx, ch) in (x..max_x).zip(text.chars()) {
        buf[(cx, y)].set_char(ch).set_style(style);
    }
}

fn hline(buf: &mut Buffer, x0: u16, x1: u16, y: u16, ch: char, style: Style) {
    for x in x0..x1 {
        buf[(x, y)].set_char(ch).set_style(style);
    }
}

pub fn draw(f: &mut Frame, s: &mut Studio) {
    let area = f.area();
    let buf = f.buffer_mut();
    buf.set_style(area, Style::default().bg(BG).fg(TEXT));
    if area.width < MIN_W || area.height < MIN_H {
        let p = Paragraph::new(format!(
            "Terminal too small: Hummr needs at least {MIN_W}×{MIN_H} characters (now {}×{}).\nMake the window bigger, or press q to quit.",
            area.width, area.height
        ))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
        let y = area.height / 2;
        f.render_widget(
            p,
            Rect::new(
                area.x,
                area.y + y.saturating_sub(1),
                area.width,
                3.min(area.height),
            ),
        );
        return;
    }
    let (w, h) = (area.width, area.height);
    let (x0, y0) = (area.x, area.y);
    s.lane_cols = w - HEADER_W - 3;
    s.visible_rows = usize::from((h - CHROME_H) / LANE_H).max(1);
    s.clamp_view();

    let border = Style::default().fg(FAINT);
    let buf = f.buffer_mut();
    let sep_x = x0 + HEADER_W + 1;
    let right = x0 + w - 1;
    // Frame.
    hline(buf, x0 + 1, right, y0, '─', border);
    buf[(x0, y0)].set_char('┌').set_style(border);
    buf[(right, y0)].set_char('┐').set_style(border);
    for y in y0 + 1..y0 + h - 1 {
        buf[(x0, y)].set_char('│').set_style(border);
        buf[(right, y)].set_char('│').set_style(border);
    }
    hline(buf, x0 + 1, right, y0 + h - 1, '─', border);
    buf[(x0, y0 + h - 1)].set_char('└').set_style(border);
    buf[(right, y0 + h - 1)].set_char('┘').set_style(border);
    for y in [y0 + 2, y0 + 5, y0 + h - 4] {
        hline(buf, x0 + 1, right, y, '─', border);
        buf[(x0, y)].set_char('├').set_style(border);
        buf[(right, y)].set_char('┤').set_style(border);
    }
    for y in y0 + 3..y0 + h - 4 {
        if y != y0 + 5 {
            buf[(sep_x, y)].set_char('│').set_style(border);
        }
    }
    buf[(sep_x, y0 + 2)].set_char('┬').set_style(border);
    buf[(sep_x, y0 + 5)].set_char('┼').set_style(border);
    buf[(sep_x, y0 + h - 4)].set_char('┴').set_style(border);

    draw_title(buf, s, Rect::new(x0 + 1, y0 + 1, w - 2, 1));
    let lanes = Rect::new(sep_x + 1, y0 + 3, s.lane_cols, h - CHROME_H + 2);
    draw_ruler(buf, s, Rect::new(lanes.x, y0 + 3, lanes.width, 2));
    let rows_area = Rect::new(x0 + 1, y0 + 6, w - 2, h - CHROME_H);
    draw_rows(buf, s, rows_area, sep_x);
    draw_status(buf, s, Rect::new(x0 + 1, y0 + h - 3, w - 2, 1));
    draw_bottom(buf, s, Rect::new(x0 + 1, y0 + h - 2, w - 2, 1));

    if s.mode == Mode::Help {
        draw_help(f, area);
    }
}

fn draw_title(buf: &mut Buffer, s: &Studio, r: Rect) {
    let song = s.song();
    let max = r.x + r.width;
    put(
        buf,
        r.x + 1,
        r.y,
        &song.name,
        max,
        Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
    );
    let key = song
        .key
        .map_or_else(|| "no key".to_string(), |k| format!("key {k}"));
    let snap = song
        .snap
        .map_or_else(|| "snap off".to_string(), |g| format!("snap {g}"));
    let info = format!(
        "♩ {} BPM · {} · {key} · {snap} ",
        hummr_core::beat::fmt_bpm(song.bpm),
        song.time_sig
    );
    let len = info.chars().count() as u16;
    let x = (max)
        .saturating_sub(len)
        .max(r.x + song.name.chars().count() as u16 + 3);
    put(buf, x, r.y, &info, max, Style::default().fg(ACCENT));
}

fn draw_ruler(buf: &mut Buffer, s: &Studio, r: Rect) {
    let song = s.song();
    let bar = song.bar_beats();
    let max = r.x + r.width;
    let cpb = s.view.cols_per_beat;
    let first = s.view.start_beat.floor() as i64;
    let last = s.view.col_to_beat(f64::from(r.width)).ceil() as i64;
    let mut free_from = r.x;
    for k in first..=last {
        let b = k as f64;
        let col = s.view.beat_to_col(b).round();
        if col < 0.0 || col >= f64::from(r.width) {
            continue;
        }
        let x = r.x + col as u16;
        let in_bar = b.rem_euclid(bar);
        if in_bar.abs() < 1e-9 {
            let label = format!("{}", (b / bar).round() as i64 + 1);
            if x >= free_from {
                put(buf, x, r.y + 1, &label, max, Style::default().fg(TEXT));
                free_from = x + label.chars().count() as u16 + 1;
            }
        } else if cpb >= 2.0 && x >= free_from {
            put(buf, x, r.y + 1, "·", max, Style::default().fg(FAINT));
        }
    }
    let ph = s.view.beat_to_col(s.playhead).floor();
    if ph >= 0.0 && ph < f64::from(r.width) {
        let color = if s.playing { GREEN } else { ACCENT };
        put(
            buf,
            r.x + ph as u16,
            r.y,
            "▼",
            max,
            Style::default().fg(color),
        );
    }
}

fn draw_rows(buf: &mut Buffer, s: &mut Studio, r: Rect, sep_x: u16) {
    let rows = s.rows();
    if rows.is_empty() {
        let msg = "No tracks yet. Press n for a new track, r to record, or i to import a sample.";
        let x = sep_x + 2;
        put(
            buf,
            x,
            r.y + 1,
            msg,
            r.x + r.width,
            Style::default().fg(DIM),
        );
        return;
    }
    let song = s.project.song.clone();
    let selected = s.selected_clip_id();
    let cur = s.current_row();
    let lane_x = sep_x + 1;
    let max = r.x + r.width;
    let cols = i64::from(s.lane_cols);
    for (vi, row) in rows
        .iter()
        .enumerate()
        .skip(s.row_scroll)
        .take(s.visible_rows)
    {
        let y = r.y + ((vi - s.row_scroll) as u16) * LANE_H;
        let is_cur = Some(*row) == cur;
        if is_cur {
            for yy in y..y + LANE_H {
                for x in lane_x..max {
                    buf[(x, yy)].set_bg(ROW_IN);
                }
            }
        }
        let track = &song.tracks[row.track];
        let audible = song.audible(row.track);
        let head_style = if is_cur {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else if audible {
            Style::default().fg(TEXT)
        } else {
            Style::default().fg(FAINT)
        };
        let label = if row.lane == 0 {
            format!("{} {}", row.track + 1, track.name)
        } else {
            format!("  ↳ lane {}", row.lane + 1)
        };
        put(buf, r.x, y + 1, &label, sep_x, head_style);
        if row.lane == 0 {
            let mut flags = String::new();
            if track.mute {
                flags.push_str(" M");
            }
            if track.solo {
                flags.push_str(" S");
            }
            if !flags.is_empty() {
                let fx = sep_x.saturating_sub(flags.chars().count() as u16);
                put(buf, fx, y, &flags, sep_x, Style::default().fg(YELLOW));
            }
        }

        let lanes = song.lanes(row.track);
        for (clip, lane) in track.clips.iter().zip(lanes) {
            if lane != row.lane {
                continue;
            }
            let a = s.view.beat_to_col(clip.start_beat).round() as i64;
            let b = (s.view.beat_to_col(clip.end_beat(song.bpm)).round() as i64 - 1).max(a);
            if b < 0 || a >= cols {
                continue;
            }
            let sel = selected == Some(clip.id);
            let base = if audible {
                track_color(row.track)
            } else {
                FAINT
            };
            let color = if sel { ACCENT } else { scale(base, 0.85) };
            let wave_color = if sel { ACCENT } else { base };
            let w = (b - a + 1) as usize;
            let levels = s
                .peaks
                .levels(clip, song.bpm, s.view.cols_per_beat, &s.project.sources)
                .to_vec();
            let (tl, tr, bl, br, hz, vt) = if sel {
                ('╔', '╗', '╚', '╝', '═', '║')
            } else {
                ('┌', '┐', '└', '┘', '─', '│')
            };
            let mut top: Vec<char> = vec![hz; w];
            let mut mid: Vec<char> = (0..w)
                .map(|i| glyph(levels.get(i).copied().unwrap_or(0.0)))
                .collect();
            let mut bot: Vec<char> = vec![hz; w];
            if w == 1 {
                top[0] = '╷';
                mid[0] = vt;
                bot[0] = '╵';
            } else {
                top[0] = tl;
                top[w - 1] = tr;
                mid[0] = vt;
                mid[w - 1] = vt;
                bot[0] = bl;
                bot[w - 1] = br;
                let name = clip_label(clip);
                if w >= 6 {
                    let room = w - 5;
                    let shown: String = name.chars().take(room).collect();
                    let text = format!(" {shown} ");
                    for (i, ch) in text.chars().enumerate() {
                        top[2 + i] = ch;
                    }
                }
            }
            for i in 0..w {
                let col = a + i as i64;
                if col < 0 || col >= cols {
                    continue;
                }
                let x = lane_x + col as u16;
                let border_style = Style::default().fg(color);
                let title = i >= 2 && i < w.saturating_sub(1) && top[i] != hz;
                let top_style = if title {
                    Style::default().fg(if sel { ACCENT } else { TEXT })
                } else {
                    border_style
                };
                buf[(x, y)].set_char(top[i]).set_style(top_style);
                let mid_style = if i == 0 || i == w - 1 {
                    border_style
                } else {
                    Style::default().fg(wave_color)
                };
                buf[(x, y + 1)].set_char(mid[i]).set_style(mid_style);
                buf[(x, y + 2)].set_char(bot[i]).set_style(border_style);
            }
        }
    }
}

fn draw_status(buf: &mut Buffer, s: &Studio, r: Rect) {
    let song = s.song();
    let max = r.x + r.width;
    let bar = song.bar_beats();
    let ph = s.playhead;
    let bar_no = (ph / bar).floor() as i64 + 1;
    let beat = ph - (bar_no - 1) as f64 * bar + 1.0;
    let end = song.beats_to_seconds(song.end_beats());
    let icon = if s.playing { "▶" } else { "■" };
    let mut text = format!(
        " {icon} {} / {}   bar {bar_no} · beat {}",
        fmt_time(song.beats_to_seconds(ph)),
        fmt_time(end),
        fmt_beats(beat)
    );
    if let Some(t) = s.current_track() {
        text.push_str(&format!("   track {} {}", t + 1, song.tracks[t].name));
        match s.selected_clip() {
            Some(c) => text.push_str(&format!(" · clip: {}", clip_label(c))),
            None => text.push_str(" · no clip here"),
        }
    }
    put(buf, r.x, r.y, &text, max, Style::default().fg(TEXT));
}

pub const HINTS: &str = "hjkl move  HJKL move clip  space play  r rec  s split  enter edit  ? help";

fn draw_bottom(buf: &mut Buffer, s: &Studio, r: Rect) {
    let max = r.x + r.width;
    match &s.mode {
        Mode::Command(text) => {
            put(
                buf,
                r.x + 1,
                r.y,
                &format!(":{text}▏"),
                max,
                Style::default().fg(TEXT),
            );
        }
        _ => match &s.message {
            Some((m, level)) => {
                let color = match level {
                    Level::Info => GREEN,
                    Level::Warn => YELLOW,
                    Level::Error => RED,
                };
                put(buf, r.x + 1, r.y, m, max, Style::default().fg(color));
            }
            None => put(buf, r.x + 1, r.y, HINTS, max, Style::default().fg(DIM)),
        },
    }
}

/// The `?` help screen, in plain words.
pub const HELP: &[(&str, &str)] = &[
    ("", "Moving around"),
    ("h / l", "move the playhead one snap step left / right"),
    ("j / k", "next / previous row (track or overlap lane)"),
    ("{ / }", "previous / next bar"),
    ("Home / End", "start / end of the song"),
    ("+ / - / 0", "zoom in / out / fit everything"),
];

fn draw_help(f: &mut Frame, area: Rect) {
    let r = crate::ui::centered(area, 80, 80, 60, 16);
    let buf = f.buffer_mut();
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            buf[(x, y)].set_char(' ').set_bg(BG).set_fg(TEXT);
        }
    }
    let mut y = r.y + 1;
    put(
        buf,
        r.x + 2,
        y,
        "Help (Esc to close)",
        r.x + r.width,
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    );
    y += 2;
    for (k, what) in HELP {
        if y >= r.y + r.height - 1 {
            break;
        }
        if k.is_empty() {
            put(
                buf,
                r.x + 2,
                y,
                what,
                r.x + r.width,
                Style::default().fg(ACCENT),
            );
        } else {
            put(buf, r.x + 4, y, k, r.x + r.width, Style::default().fg(TEXT));
            put(
                buf,
                r.x + 18,
                y,
                what,
                r.x + r.width - 1,
                Style::default().fg(DIM),
            );
        }
        y += 1;
    }
}
