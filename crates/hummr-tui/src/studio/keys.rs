//! Keyboard handling for the timeline.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{Mode, Studio, command};

pub fn on_key(s: &mut Studio, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && key.code == KeyCode::Char('c') {
        match s.mode {
            Mode::Normal => s.request_quit(false),
            _ => s.mode = Mode::Normal,
        }
        return;
    }
    match s.mode.clone() {
        Mode::Normal => normal(s, key, ctrl),
        Mode::Command(text) => command_line(s, key, text),
        Mode::Help => {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Enter
            ) {
                s.mode = Mode::Normal;
            }
        }
    }
}

fn normal(s: &mut Studio, key: KeyEvent, _ctrl: bool) {
    // Any key clears an old message.
    s.message = None;
    let step = s.song().snap_beats();
    match key.code {
        KeyCode::Char('h') | KeyCode::Left => s.seek(s.playhead - step),
        KeyCode::Char('l') | KeyCode::Right => s.seek(s.playhead + step),
        KeyCode::Char('j') | KeyCode::Down => s.move_row(1),
        KeyCode::Char('k') | KeyCode::Up => s.move_row(-1),
        KeyCode::Char('{') => s.bar_jump(-1),
        KeyCode::Char('}') => s.bar_jump(1),
        KeyCode::Home => s.seek(0.0),
        KeyCode::End => {
            let end = s.song().end_beats();
            s.seek(end);
        }
        KeyCode::Char('+') | KeyCode::Char('=') => s.zoom(1),
        KeyCode::Char('-') | KeyCode::Char('_') => s.zoom(-1),
        KeyCode::Char('0') => s.fit(),
        KeyCode::Char(' ') => s.toggle_play(),
        KeyCode::Char('?') => s.mode = Mode::Help,
        KeyCode::Char(':') => s.mode = Mode::Command(String::new()),
        KeyCode::Char('q') => s.request_quit(false),
        _ => {}
    }
}

fn command_line(s: &mut Studio, key: KeyEvent, mut text: String) {
    match key.code {
        KeyCode::Esc => s.mode = Mode::Normal,
        KeyCode::Enter => {
            s.mode = Mode::Normal;
            command::run(s, &text);
        }
        KeyCode::Backspace => {
            if text.pop().is_none() {
                s.mode = Mode::Normal;
            } else {
                s.mode = Mode::Command(text);
            }
        }
        KeyCode::Char(c) => {
            text.push(c);
            s.mode = Mode::Command(text);
        }
        _ => {}
    }
}
