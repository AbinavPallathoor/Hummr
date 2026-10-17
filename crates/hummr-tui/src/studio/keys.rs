//! Keyboard handling for the timeline.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::inspector::Panel;
use super::{Mode, Studio, command};

pub fn on_key(s: &mut Studio, key: KeyEvent) {
    if s.notes_editor.is_some() {
        s.notes_key(key);
        return;
    }
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
        Mode::Recording => match key.code {
            KeyCode::Char('r') | KeyCode::Char(' ') | KeyCode::Enter => s.stop_recording(true),
            KeyCode::Esc => s.stop_recording(false),
            _ => {}
        },
        Mode::Import(mut p) => match key.code {
            KeyCode::Esc | KeyCode::Char('q') => s.mode = Mode::Normal,
            KeyCode::Char('j') | KeyCode::Down => {
                p.sel = (p.sel + 1).min(p.items.len().saturating_sub(1));
                s.mode = Mode::Import(p);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                p.sel = p.sel.saturating_sub(1);
                s.mode = Mode::Import(p);
            }
            KeyCode::Enter => {
                s.mode = Mode::Normal;
                if let Some(item) = p.items.get(p.sel).cloned() {
                    s.import_item(&item);
                }
            }
            _ => {}
        },
        Mode::Command(text) => command_line(s, key, text),
        Mode::Inspector => inspector(s, key, ctrl),
        Mode::Mixer => mixer(s, key, ctrl),
        Mode::TempoPrompt(p) => match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                s.mode = Mode::TempoPrompt(super::tempo_prompt::TempoPrompt {
                    sel: (p.sel + 1).min(2),
                    ..p
                });
            }
            KeyCode::Char('k') | KeyCode::Up => {
                s.mode = Mode::TempoPrompt(super::tempo_prompt::TempoPrompt {
                    sel: p.sel.saturating_sub(1),
                    ..p
                });
            }
            KeyCode::Char(c @ '1'..='3') => s.prompt_choose(c as usize - '1' as usize),
            KeyCode::Char('h') => s.prompt_scale(0.5),
            KeyCode::Char('d') => s.prompt_scale(2.0),
            KeyCode::Enter => s.prompt_choose(p.sel),
            KeyCode::Esc => s.prompt_choose(2),
            _ => {}
        },
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

fn normal(s: &mut Studio, key: KeyEvent, ctrl: bool) {
    // Any key clears an old message.
    s.message = None;
    let step = s.song().snap_beats();
    if ctrl {
        if key.code == KeyCode::Char('r') {
            s.redo();
        }
        return;
    }
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
        KeyCode::Char('H') => s.nudge_clip(-1.0),
        KeyCode::Char('L') => s.nudge_clip(1.0),
        KeyCode::Char('J') => s.shift_clip_track(1),
        KeyCode::Char('K') => s.shift_clip_track(-1),
        KeyCode::Char('s') => s.split(),
        KeyCode::Char('x') | KeyCode::Delete => s.delete_clip(),
        KeyCode::Char('y') => s.copy_clip(),
        KeyCode::Char('p') => s.paste(),
        KeyCode::Char('d') => s.duplicate(),
        KeyCode::Char('u') => s.undo(),
        KeyCode::Char('m') => s.toggle_mute(),
        KeyCode::Char('S') => s.toggle_solo(),
        KeyCode::Char('n') => {
            s.new_track(None);
        }
        KeyCode::Char('r') => s.start_recording(),
        KeyCode::Char('i') => s.open_import(),
        KeyCode::Enter => s.open_inspector(),
        KeyCode::Char('<') | KeyCode::Char(',') => s.nudge_bpm(-1.0),
        KeyCode::Char('>') | KeyCode::Char('.') => s.nudge_bpm(1.0),
        KeyCode::Char('T') => s.tap_tempo(),
        KeyCode::Char('M') => s.toggle_mixer(),
        KeyCode::Char('e') => s.open_notes(),
        KeyCode::Char('?') => s.mode = Mode::Help,
        KeyCode::Char(':') => s.mode = Mode::Command(String::new()),
        KeyCode::Char('q') => s.request_quit(false),
        _ => {}
    }
}

fn mixer(s: &mut Studio, key: KeyEvent, ctrl: bool) {
    s.message = None;
    if ctrl {
        if key.code == KeyCode::Char('r') {
            s.redo();
        }
        return;
    }
    match key.code {
        KeyCode::Esc => s.mode = Mode::Normal,
        KeyCode::Char('M') | KeyCode::Char('q') => s.toggle_mixer(),
        KeyCode::Char('h') | KeyCode::Left => s.select_track(-1),
        KeyCode::Char('l') | KeyCode::Right => s.select_track(1),
        KeyCode::Char('k') | KeyCode::Up | KeyCode::Char('+') | KeyCode::Char('=') => {
            s.nudge_volume(1.0)
        }
        KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('-') => s.nudge_volume(-1.0),
        KeyCode::Char('[') => s.nudge_pan(-0.1),
        KeyCode::Char(']') => s.nudge_pan(0.1),
        KeyCode::Char('0') => s.reset_strip(),
        KeyCode::Char('m') => s.toggle_mute(),
        KeyCode::Char('S') => s.toggle_solo(),
        KeyCode::Char(' ') => s.toggle_play(),
        KeyCode::Char('u') => s.undo(),
        KeyCode::Char('?') => s.mode = Mode::Help,
        _ => {}
    }
}

fn inspector(s: &mut Studio, key: KeyEvent, ctrl: bool) {
    s.message = None;
    let has_sound = s.engine.has_sound();
    let Some(ins) = s.inspector.as_mut() else {
        s.mode = Mode::Normal;
        return;
    };
    let n = ins.panel.controls().len();
    let panel_index = Panel::ALL.iter().position(|&p| p == ins.panel).unwrap_or(0);
    let set_panel = |ins: &mut super::inspector::Inspector, i: usize| {
        ins.panel = Panel::ALL[i % Panel::ALL.len()];
        ins.row = 0;
    };
    match key.code {
        KeyCode::Char('r') if ctrl => {
            s.redo();
            after_history(s);
        }
        _ if ctrl => {}
        KeyCode::Esc | KeyCode::Char('q') => s.close_inspector(),
        KeyCode::Tab => set_panel(ins, panel_index + 1),
        KeyCode::BackTab => set_panel(ins, panel_index + Panel::ALL.len() - 1),
        KeyCode::Char(c @ '1'..='3') => set_panel(ins, c as usize - '1' as usize),
        KeyCode::Char('j') | KeyCode::Down => {
            ins.row = (ins.row + 1).min(n - 1);
            s.last_merge = None;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            ins.row = ins.row.saturating_sub(1);
            s.last_merge = None;
        }
        KeyCode::Char('h') | KeyCode::Left => s.inspector_adjust(-1.0),
        KeyCode::Char('l') | KeyCode::Right => s.inspector_adjust(1.0),
        KeyCode::Enter => s.inspector_apply(),
        KeyCode::Char(' ') => {
            if !has_sound {
                s.say(
                    "No sound device, so there is nothing to hear. Changes still apply.",
                    crate::app::Level::Warn,
                );
                return;
            }
            ins.preview = !ins.preview;
            s.refresh_preview();
        }
        KeyCode::Char('u') => {
            s.undo();
            after_history(s);
        }
        _ => {}
    }
}

/// After undo/redo in the inspector: close it if its clip is gone, else
/// hear the change.
fn after_history(s: &mut Studio) {
    let id = s.inspector.as_ref().map(|i| i.clip);
    if id.is_some_and(|id| s.song().clip(id).is_none()) {
        s.close_inspector();
    } else {
        s.refresh_preview();
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
