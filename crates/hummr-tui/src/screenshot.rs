//! Renders editor screens to SVG, for the README. Not part of the app.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

use crate::app::{App, Focus, Lower, Mode, Options};
use crate::ui;

const DEFAULT_FG: (u8, u8, u8) = (222, 222, 232);
const DEFAULT_BG: (u8, u8, u8) = (15, 15, 21);

fn rgb(c: Color, default: (u8, u8, u8)) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::White => (255, 255, 255),
        Color::Black => (0, 0, 0),
        Color::Gray => (170, 170, 170),
        Color::DarkGray => (90, 90, 90),
        Color::Red => (220, 70, 70),
        Color::Green => (90, 200, 120),
        Color::Yellow => (230, 200, 90),
        Color::Blue => (90, 130, 230),
        Color::Cyan => (80, 200, 220),
        Color::Magenta => (200, 110, 220),
        _ => default,
    }
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Converts a rendered buffer to an SVG that looks like a terminal window.
pub fn to_svg(buf: &Buffer, title: &str) -> String {
    let (cw, ch) = (9.0f64, 19.0f64);
    let (pad, bar) = (14.0, 34.0);
    let w = f64::from(buf.area.width) * cw + 2.0 * pad;
    let h = f64::from(buf.area.height) * ch + 2.0 * pad + bar;
    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
<rect width="{w}" height="{h}" rx="10" fill="#0b0b10"/>
<circle cx="20" cy="17" r="6" fill="#ff5f57"/><circle cx="40" cy="17" r="6" fill="#febc2e"/><circle cx="60" cy="17" r="6" fill="#28c840"/>
<text x="{tx}" y="22" fill="#77778a" font-family="-apple-system, Segoe UI, Helvetica, Arial, sans-serif" font-size="13" text-anchor="middle">{title}</text>
<g font-family="JetBrains Mono, SF Mono, Menlo, Consolas, DejaVu Sans Mono, monospace" font-size="15">
"##,
        tx = w / 2.0,
        title = escape(title),
    );
    let ox = pad;
    let oy = pad + bar - 6.0;
    for y in 0..buf.area.height {
        let ry = oy + f64::from(y) * ch;
        // Backgrounds, merged into runs.
        let mut x = 0;
        while x < buf.area.width {
            let bg = rgb(buf[(x, y)].bg, DEFAULT_BG);
            let mut end = x + 1;
            while end < buf.area.width && rgb(buf[(end, y)].bg, DEFAULT_BG) == bg {
                end += 1;
            }
            if bg != DEFAULT_BG {
                let _ = writeln!(
                    s,
                    r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{ch}" fill="{}"/>"#,
                    ox + f64::from(x) * cw,
                    ry,
                    f64::from(end - x) * cw,
                    hex(bg)
                );
            }
            x = end;
        }
        // Text, one element per cell run of identical style, stretched to
        // the exact cell width so columns line up in any font.
        let mut x = 0;
        while x < buf.area.width {
            let cell = &buf[(x, y)];
            let fg = rgb(cell.fg, DEFAULT_FG);
            let bold = cell.modifier.contains(Modifier::BOLD);
            let mut text = String::new();
            let mut end = x;
            while end < buf.area.width {
                let c = &buf[(end, y)];
                if rgb(c.fg, DEFAULT_FG) != fg || c.modifier.contains(Modifier::BOLD) != bold {
                    break;
                }
                text.push_str(c.symbol());
                end += 1;
            }
            if !text.trim().is_empty() {
                let _ = writeln!(
                    s,
                    r#"<text x="{:.1}" y="{:.1}" fill="{}"{} textLength="{:.1}" lengthAdjust="spacingAndGlyphs" xml:space="preserve">{}</text>"#,
                    ox + f64::from(x) * cw,
                    ry + ch * 0.74,
                    hex(fg),
                    if bold { r#" font-weight="bold""# } else { "" },
                    f64::from(end - x) * cw,
                    escape(&text)
                );
            }
            x = end.max(x + 1);
        }
    }
    s.push_str("</g>\n</svg>\n");
    s
}

fn draw(app: &mut App, w: u16, h: u16) -> Buffer {
    let mut term = Terminal::new(TestBackend::new(w, h)).expect("test backend");
    term.draw(|f| ui::draw(f, app, Duration::from_millis(16)))
        .expect("draw");
    term.backend().buffer().clone()
}

/// A small three-track demo song built from a hummed take.
fn demo_song(take: &Path) -> App {
    let mut app = App::new(&Options {
        file: None,
        animations: false,
        mouse: false,
    });
    app.open(take);
    app.wait_for_job();
    app.save_path = Some("twinkle.mid".into());
    // Track 2: a cello bass line, typed in.
    app.execute(crate::command::Cmd::Track("new cello".into()));
    app.type_notes_at(
        0.0,
        "C3 1/2 E3 1/2 F3 1/2 C3 1/2 A2 1/2 C3 1/2 G2 1/2 C3 1/2",
    );
    // Track 3: a flute harmony a third above for the first phrase.
    app.execute(crate::command::Cmd::Track("new flute".into()));
    app.type_notes_at(0.0, "E5 1/4 E5 1/4 B5 1/4 B5 1/4 C6 1/4 C6 1/4 B5 1/2");
    app.select_track(0);
    app.selected = 4;
    app.cursor = 4.0;
    app.message = None;
    app
}

/// Renders the README screenshots into `dir` as SVG files.
pub fn write_all(take: &Path, dir: &Path, w: u16, h: u16) -> std::io::Result<Vec<String>> {
    std::fs::create_dir_all(dir)?;
    let mut written = Vec::new();
    let mut save = |name: &str, title: &str, buf: Buffer| -> std::io::Result<()> {
        let path = dir.join(format!("{name}.svg"));
        std::fs::write(&path, to_svg(&buf, title))?;
        written.push(path.display().to_string());
        Ok(())
    };

    // 1. Editing a three-track song while it plays.
    let mut app = demo_song(take);
    app.playing = true;
    app.playhead.value = 6.5;
    for (i, db) in [-9.0f32, -14.0, -18.0].into_iter().enumerate() {
        if let Some(s) = app.strips.get_mut(i) {
            s.meter.level_db = db;
            s.meter.peak_db = db + 3.0;
        }
    }
    app.voice.meter.level_db = -20.0;
    app.voice.meter.peak_db = -16.0;
    app.master.level_db = -7.0;
    app.master.peak_db = -4.0;
    app.info("14 notes · C major · 100 BPM · space plays · r adds a layer · R redoes this track");
    save(
        "editor",
        "hummr — editing a three-track song",
        draw(&mut app, w, h),
    )?;

    // 2. Recording a new layer: live pitch meter and contour.
    let mut app = demo_song(take);
    app.demo_recording = true;
    app.record_seconds = 4.2;
    app.record_beat = Some(7.0);
    app.live_pitch.value = Some(71.08);
    app.live_hz = Some(495.8);
    app.mic.level_db = -11.0;
    app.mic.peak_db = -6.0;
    app.mode = Mode::Recording;
    // A harmony line hummed over bar 2 (C5 C5 B4 B4), ending on the note
    // the pitch meter shows.
    app.contour = (0..140)
        .map(|i| {
            let b = 4.0 + f64::from(i) * 0.0215;
            let note = [72.0f32, 72.0, 71.0, 71.0][((b - 4.0) as usize).min(3)];
            (b, Some(note + 0.12 * (b as f32 * 11.0).sin()))
        })
        .collect();
    app.view.left_beat = 0.0;
    app.info(
        "recording from MacBook Microphone: new layer from bar 2 beat 1 (the song plays along)",
    );
    save(
        "recording",
        "hummr — humming a new layer",
        draw(&mut app, w, h),
    )?;

    // 3. Fuzzy instrument picker.
    let mut app = demo_song(take);
    app.open_picker();
    app.picker_query = "vio".into();
    app.refresh_picker();
    save(
        "instruments",
        "hummr — picking an instrument",
        draw(&mut app, w, h),
    )?;

    // 4. Why notes changed.
    let mut app = demo_song(take);
    app.lower = Lower::Why;
    app.focus = Focus::Tracker;
    save(
        "why",
        "hummr — why each note was changed",
        draw(&mut app, w, h),
    )?;

    // 5. Help.
    let mut app = demo_song(take);
    app.mode = Mode::Help;
    save("help", "hummr — help", draw(&mut app, w, h))?;

    Ok(written)
}
