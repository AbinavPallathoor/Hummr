//! Drawing the timeline. Everything is drawn from `Studio` state.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Paragraph, Wrap};

use super::mixer::{MIXER_H, STRIP_W, fmt_db, fmt_pan, meter_cells};
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
    let mixer_h = if s.show_mixer { MIXER_H } else { 0 };
    s.visible_rows = usize::from((h - CHROME_H - mixer_h) / LANE_H).max(1);
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
    let rows_area = Rect::new(x0 + 1, y0 + 6, w - 2, h - CHROME_H - mixer_h);
    draw_rows(buf, s, rows_area, sep_x);
    if s.show_mixer {
        let r = Rect::new(x0, y0 + 6 + rows_area.height, w, MIXER_H);
        draw_mixer(buf, s, r, sep_x);
    }
    draw_recording(buf, s, rows_area, sep_x + 1);
    draw_status(buf, s, Rect::new(x0 + 1, y0 + h - 3, w - 2, 1));
    draw_bottom(buf, s, Rect::new(x0 + 1, y0 + h - 2, w - 2, 1));

    match &s.mode {
        Mode::Help => draw_help(f, area),
        Mode::Import(p) => {
            let p = p.clone();
            draw_import(f, area, &p);
        }
        Mode::Inspector => draw_inspector(f, area, s),
        Mode::TempoPrompt(p) => draw_tempo_prompt(f, area, s, p),
        _ => {}
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

/// The take being recorded: a red box growing from where it started.
fn draw_recording(buf: &mut Buffer, s: &Studio, r: Rect, lane_x: u16) {
    let Some(rec) = &s.recording else { return };
    let rows = s.rows();
    let Some(vi) = rows
        .iter()
        .position(|row| row.track == rec.track)
        .filter(|&vi| vi >= s.row_scroll && vi < s.row_scroll + s.visible_rows)
    else {
        return;
    };
    let y = r.y + ((vi - s.row_scroll) as u16) * LANE_H;
    let a = s.view.beat_to_col(rec.at_beat).round() as i64;
    let b = (s.view.beat_to_col(rec.beat).round() as i64).max(a);
    let cols = i64::from(s.lane_cols);
    let style = Style::default().fg(RED);
    for col in a.max(0)..=b.min(cols - 1) {
        let x = lane_x + col as u16;
        let (top, mid, bot) = if col == a {
            ('┌', '│', '└')
        } else if col == b {
            ('┐', '│', '┘')
        } else {
            ('─', glyph(rec.level), '─')
        };
        buf[(x, y)].set_char(top).set_style(style);
        buf[(x, y + 1)].set_char(mid).set_style(style);
        buf[(x, y + 2)].set_char(bot).set_style(style);
    }
    if b - a >= 8 && a + 2 < cols && a >= 0 {
        put(
            buf,
            lane_x + a as u16 + 2,
            y,
            " ● rec ",
            lane_x + b.min(cols - 1) as u16,
            style,
        );
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
    if let Some(rec) = &s.recording {
        let text = match rec.count_left {
            Some(n) => format!(" ● count-in: {n}…  get ready"),
            None => format!(
                " ● REC  {}  bar {} · beat {}",
                fmt_time(song.beats_to_seconds((rec.beat - rec.at_beat).max(0.0))),
                (rec.beat / bar).floor() as i64 + 1,
                fmt_beats((rec.beat - (rec.beat / bar).floor() * bar + 1.0).floor())
            ),
        };
        put(
            buf,
            r.x,
            r.y,
            &text,
            max,
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        );
        return;
    }
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

pub const MIXER_HINTS: &str =
    "h/l track  j/k volume  [ ] pan  m mute  S solo  0 reset  M hide  esc clips";
pub const INSPECTOR_HINTS: &str =
    "tab panel  j/k choose  h/l change  enter apply  space loop  u undo  esc close";

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
        Mode::Mixer if s.message.is_none() => put(
            buf,
            r.x + 1,
            r.y,
            MIXER_HINTS,
            max,
            Style::default().fg(DIM),
        ),
        Mode::Inspector if s.message.is_none() => put(
            buf,
            r.x + 1,
            r.y,
            INSPECTOR_HINTS,
            max,
            Style::default().fg(DIM),
        ),
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
    ("j / k", "next / previous row (a track, or an overlap lane)"),
    ("{ / }", "previous / next bar"),
    ("Home / End", "start / end of the song"),
    ("+ / - / 0", "zoom in / zoom out / fit the whole song"),
    ("", "Playing and recording"),
    ("space", "play / stop from the playhead"),
    (
        "r",
        "record on this track at the playhead (one bar of clicks first)",
    ),
    ("i", "import a sample (built-in sounds or WAV files)"),
    ("", "Clips (the clip under the playhead on this row)"),
    ("H / L", "move the clip one snap step earlier / later"),
    ("J / K", "move the clip to the track below / above"),
    ("s", "split the clip at the playhead"),
    ("x", "delete the clip"),
    ("y / p", "copy the clip / paste it at the playhead"),
    ("d", "duplicate the clip right after itself"),
    (
        "Enter",
        "open the clip's inspector: turn into, effects, fix",
    ),
    ("e", "edit the notes of a notes clip (Esc comes back)"),
    ("", "Tracks and song"),
    ("n", "new track"),
    ("m / S", "mute / solo this track"),
    ("M", "show or hide the mixer"),
    ("< / >", "tempo down / up by 1 BPM"),
    ("T", "tap the tempo (press a few times in time)"),
    ("u / Ctrl-r", "undo / redo"),
    ("?", "this help"),
    (":", "type a command (below)"),
    ("q", "quit"),
    ("", "Commands"),
    (":bpm 120", "set the tempo (also :bpm half, :bpm double)"),
    (":time 3/4", "time signature (4/4 or 3/4)"),
    (":key A minor", "song key (or :key none)"),
    (":snap 1/8", "snap size: 1/4, 1/8, 1/16, 1/8t or off"),
    (
        ":track name X",
        "rename this track (:track delete removes it)",
    ),
    (
        ":w [file]",
        "save (to a new file if given); :q quits, :q! without saving",
    ),
    (
        ":export x.wav",
        "export the song as WAV (or .mid for the notes clips)",
    ),
    (":anim on|off", "animations"),
];

fn draw_help(f: &mut Frame, area: Rect) {
    let r = crate::ui::centered(area, 92, 92, 76, 20);
    let buf = f.buffer_mut();
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            buf[(x, y)].set_char(' ').set_bg(BG).set_fg(TEXT);
        }
    }
    let border = Style::default().fg(ACCENT);
    hline(buf, r.x, r.x + r.width, r.y, '─', border);
    hline(buf, r.x, r.x + r.width, r.y + r.height - 1, '─', border);
    put(
        buf,
        r.x + 2,
        r.y,
        " Help · Esc to close ",
        r.x + r.width,
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    );
    // Two columns when the list doesn't fit in one.
    let rows = usize::from(r.height.saturating_sub(2));
    let cols = if HELP.len() > rows && r.width >= 100 {
        2
    } else {
        1
    };
    let col_w = r.width / cols as u16;
    for (i, (k, what)) in HELP.iter().enumerate() {
        let c = i / rows.max(1);
        if c >= cols {
            break;
        }
        let x = r.x + 1 + c as u16 * col_w;
        let y = r.y + 1 + (i % rows.max(1)) as u16;
        let max = x + col_w - 1;
        if k.is_empty() {
            put(buf, x + 1, y, what, max, Style::default().fg(ACCENT));
        } else {
            put(buf, x + 2, y, k, max, Style::default().fg(TEXT));
            put(buf, x + 17, y, what, max, Style::default().fg(DIM));
        }
    }
}

fn draw_import(f: &mut Frame, area: Rect, p: &super::import::Picker) {
    let r = crate::ui::centered(area, 60, 70, 50, 14);
    let buf = f.buffer_mut();
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            buf[(x, y)].set_char(' ').set_bg(BG).set_fg(TEXT);
        }
    }
    let border = Style::default().fg(ACCENT);
    hline(buf, r.x, r.x + r.width, r.y, '─', border);
    hline(buf, r.x, r.x + r.width, r.y + r.height - 1, '─', border);
    put(
        buf,
        r.x + 2,
        r.y,
        " Import a sample · j/k choose · Enter add · Esc close ",
        r.x + r.width,
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    );
    let rows = usize::from(r.height.saturating_sub(2));
    let first = p.sel.saturating_sub(rows.saturating_sub(1));
    for (i, item) in p.items.iter().enumerate().skip(first).take(rows) {
        let y = r.y + 1 + (i - first) as u16;
        let sel = i == p.sel;
        let style = if sel {
            Style::default().fg(BG).bg(ACCENT)
        } else {
            Style::default().fg(TEXT)
        };
        let text = format!("{} {}", if sel { "▸" } else { " " }, item.label());
        put(buf, r.x + 2, y, &text, r.x + r.width - 2, style);
    }
}

fn draw_inspector(f: &mut Frame, area: Rect, s: &Studio) {
    use super::inspector::{Panel, describe};
    let Some(ins) = &s.inspector else { return };
    let Some(clip) = s.song().clip(ins.clip) else {
        return;
    };
    // Tall enough for the longest panel, centred.
    let w = (area.width * 7 / 10).max(64).min(area.width);
    let h = 20.min(area.height);
    let r = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    let buf = f.buffer_mut();
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            buf[(x, y)].set_char(' ').set_bg(BG).set_fg(TEXT);
        }
    }
    let max = r.x + r.width - 1;
    let border = Style::default().fg(ACCENT);
    hline(buf, r.x, r.x + r.width, r.y, '─', border);
    hline(buf, r.x, r.x + r.width, r.y + r.height - 1, '─', border);
    for y in r.y..r.y + r.height {
        buf[(r.x, y)].set_char('│').set_style(border);
        buf[(max, y)].set_char('│').set_style(border);
    }
    for (x, y, c) in [
        (r.x, r.y, '╭'),
        (max, r.y, '╮'),
        (r.x, r.y + r.height - 1, '╰'),
        (max, r.y + r.height - 1, '╯'),
    ] {
        buf[(x, y)].set_char(c).set_style(border);
    }
    put(
        buf,
        r.x + 2,
        r.y,
        &format!(" {} · Esc to close ", clip_label(clip)),
        max,
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    );
    let preview = if !s.engine.has_sound() {
        "no sound device"
    } else if ins.preview {
        "▶ looping (space stops)"
    } else {
        "space: loop it"
    };
    let pw = preview.chars().count() as u16;
    put(
        buf,
        max.saturating_sub(pw + 2),
        r.y,
        preview,
        max,
        Style::default().fg(if ins.preview { GREEN } else { DIM }),
    );
    // Panel tabs.
    let mut x = r.x + 2;
    for (i, p) in Panel::ALL.iter().enumerate() {
        let text = format!(" {} {} ", i + 1, p.title());
        let style = if *p == ins.panel {
            Style::default()
                .fg(BG)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(DIM)
        };
        put(buf, x, r.y + 1, &text, max, style);
        x += text.chars().count() as u16 + 1;
    }
    // Controls, scrolled to keep the selected one in view.
    let list_h = usize::from(r.height.saturating_sub(5));
    let controls = ins.panel.controls();
    let first = ins.row.saturating_sub(list_h.saturating_sub(1));
    let label_w = 32.min(r.width / 2);
    for (i, ctl) in controls.iter().enumerate().skip(first).take(list_h) {
        let y = r.y + 3 + (i - first) as u16;
        let (label, value, _) = describe(s, ins, *ctl);
        let sel = i == ins.row;
        let style = if sel {
            Style::default().fg(BG).bg(ACCENT)
        } else {
            Style::default().fg(TEXT)
        };
        if sel {
            hline(buf, r.x + 1, max, y, ' ', style);
        }
        put(
            buf,
            r.x + 2,
            y,
            &format!("{} {label}", if sel { "▸" } else { " " }),
            max,
            style,
        );
        put(
            buf,
            r.x + 3 + label_w,
            y,
            &value,
            max,
            if sel {
                style
            } else {
                Style::default().fg(YELLOW)
            },
        );
    }
    let (_, _, hint) = describe(s, ins, ins.control());
    put(
        buf,
        r.x + 2,
        r.y + r.height - 2,
        hint,
        max,
        Style::default().fg(DIM),
    );
}

fn draw_tempo_prompt(f: &mut Frame, area: Rect, s: &Studio, p: &super::tempo_prompt::TempoPrompt) {
    let w = 72.min(area.width);
    let h = 11.min(area.height);
    let r = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    let buf = f.buffer_mut();
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            buf[(x, y)].set_char(' ').set_bg(BG).set_fg(TEXT);
        }
    }
    let max = r.x + r.width - 1;
    let border = Style::default().fg(ACCENT);
    hline(buf, r.x, r.x + r.width, r.y, '─', border);
    hline(buf, r.x, r.x + r.width, r.y + r.height - 1, '─', border);
    for y in r.y..r.y + r.height {
        buf[(r.x, y)].set_char('│').set_style(border);
        buf[(max, y)].set_char('│').set_style(border);
    }
    for (x, y, c) in [
        (r.x, r.y, '╭'),
        (max, r.y, '╮'),
        (r.x, r.y + r.height - 1, '╰'),
        (max, r.y + r.height - 1, '╯'),
    ] {
        buf[(x, y)].set_char(c).set_style(border);
    }
    put(
        buf,
        r.x + 2,
        r.y,
        " A different tempo ",
        max,
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    );
    let song_bpm = s.song().bpm;
    put(
        buf,
        r.x + 2,
        r.y + 2,
        &p.heard(),
        max,
        Style::default().fg(TEXT),
    );
    put(
        buf,
        r.x + 2,
        r.y + 3,
        &format!(
            "The song is {} BPM. What should happen?",
            hummr_core::beat::fmt_bpm(song_bpm)
        ),
        max,
        Style::default().fg(TEXT),
    );
    for (i, (label, what)) in p.choices(song_bpm).iter().enumerate() {
        let y = r.y + 5 + i as u16;
        let sel = i == p.sel;
        let style = if sel {
            Style::default().fg(BG).bg(ACCENT)
        } else {
            Style::default().fg(TEXT)
        };
        let text = format!("{} {} {label:<22}", if sel { "▸" } else { " " }, i + 1);
        put(buf, r.x + 2, y, &text, max, style);
        put(buf, r.x + 29, y, what, max, Style::default().fg(DIM));
    }
    put(
        buf,
        r.x + 2,
        r.y + r.height - 2,
        "j/k choose · Enter ok · h half · d double the heard tempo · Esc leave",
        max,
        Style::default().fg(DIM),
    );
}

/// Mixer strips across the bottom of the track area; `r` spans the whole
/// width including the frame columns, its first line is the separator.
fn draw_mixer(buf: &mut Buffer, s: &Studio, r: Rect, sep_x: u16) {
    let border = Style::default().fg(FAINT);
    let right = r.x + r.width - 1;
    for y in r.y + 1..r.y + r.height {
        for x in r.x + 1..right {
            buf[(x, y)]
                .set_char(' ')
                .set_style(Style::default().bg(BG).fg(TEXT));
        }
    }
    hline(buf, r.x + 1, right, r.y, '─', border);
    buf[(r.x, r.y)].set_char('├').set_style(border);
    buf[(right, r.y)].set_char('┤').set_style(border);
    buf[(sep_x, r.y)].set_char('┴').set_style(border);
    let focused = s.mode == Mode::Mixer;
    put(
        buf,
        r.x + 2,
        r.y,
        if focused {
            " Mixer · j/k volume · [ ] pan · m mute · S solo · Esc clips "
        } else {
            " Mixer · M to hide "
        },
        right,
        Style::default().fg(if focused { ACCENT } else { DIM }),
    );
    let song = s.song();
    let cur = s.current_track();
    // Strips: tracks then master, scrolled to keep the selected one in view.
    let fit = usize::from((r.width - 2) / STRIP_W).max(1);
    let n = song.tracks.len() + 1;
    let first = cur
        .unwrap_or(0)
        .saturating_sub(fit.saturating_sub(2))
        .min(n.saturating_sub(fit));
    let cells = usize::from(STRIP_W) - 3;
    for (k, i) in (first..n).take(fit).enumerate() {
        let x = r.x + 1 + k as u16 * STRIP_W;
        let max = (x + STRIP_W - 1).min(right);
        let (name, peak, vol, pan, mute, solo, audible, color) = if i < song.tracks.len() {
            let t = &song.tracks[i];
            (
                format!("{} {}", i + 1, t.name),
                s.meters.get(i).copied().unwrap_or(0.0),
                t.volume_db,
                t.pan,
                t.mute,
                t.solo,
                song.audible(i),
                track_color(i),
            )
        } else {
            (
                "Master".to_string(),
                s.master_meter,
                0.0,
                0.0,
                false,
                false,
                true,
                ACCENT,
            )
        };
        let sel = Some(i) == cur;
        let name_style = if sel && focused {
            Style::default()
                .fg(BG)
                .bg(color)
                .add_modifier(Modifier::BOLD)
        } else if sel {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(if audible { color } else { DIM })
        };
        put(
            buf,
            x + 1,
            r.y + 1,
            &format!("{name:<w$}", w = cells + 1),
            max,
            name_style,
        );
        let lit = meter_cells(peak, cells);
        for c in 0..cells {
            let frac = (c + 1) as f32 / cells as f32;
            let on = c < lit;
            let col = if !on {
                FAINT
            } else if frac > 0.92 {
                RED
            } else if frac > 0.75 {
                YELLOW
            } else {
                GREEN
            };
            buf[(x + 1 + c as u16, r.y + 2)]
                .set_char(if on { '▮' } else { '·' })
                .set_style(Style::default().fg(col));
        }
        if i < song.tracks.len() {
            put(
                buf,
                x + 1,
                r.y + 3,
                &format!("{:<7}{}", fmt_db(vol), fmt_pan(pan)),
                max,
                Style::default().fg(TEXT),
            );
            put(
                buf,
                x + 1,
                r.y + 4,
                "M",
                max,
                if mute {
                    Style::default().fg(BG).bg(YELLOW)
                } else {
                    Style::default().fg(DIM)
                },
            );
            put(
                buf,
                x + 3,
                r.y + 4,
                "S",
                max,
                if solo {
                    Style::default().fg(BG).bg(GREEN)
                } else {
                    Style::default().fg(DIM)
                },
            );
        }
    }
}
