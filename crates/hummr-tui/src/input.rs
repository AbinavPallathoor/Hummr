//! Keyboard and mouse handling: vim-like keys in normal mode.

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;

use crate::app::{App, Focus, Mode};

pub fn on_key(app: &mut App, key: KeyEvent) {
    app.view_follow = true;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && key.code == KeyCode::Char('c') {
        // Ctrl-C always gets you out of whatever you are in.
        match app.mode {
            Mode::Recording => app.stop_recording(false),
            Mode::Normal => app.execute(crate::command::Cmd::Quit { force: false }),
            _ => app.mode = Mode::Normal,
        }
        return;
    }
    match app.mode {
        Mode::Normal => normal(app, key),
        Mode::Command => command(app, key),
        Mode::Picker => picker(app, key, ctrl),
        Mode::Visual => visual(app, key),
        Mode::Recording => match key.code {
            KeyCode::Char('r') | KeyCode::Char(' ') | KeyCode::Enter => app.stop_recording(true),
            KeyCode::Esc => app.stop_recording(false),
            _ => {}
        },
        Mode::Help => {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Enter
            ) {
                app.mode = Mode::Normal;
            }
        }
    }
}

fn normal(app: &mut App, key: KeyEvent) {
    // Keys that work regardless of focus.
    match key.code {
        KeyCode::Char(':') => return app.open_command(),
        KeyCode::Char('?') => return app.mode = Mode::Help,
        KeyCode::Char('q') => return app.execute(crate::command::Cmd::Quit { force: false }),
        KeyCode::Char('r') => return app.press_record(),
        KeyCode::Char('R') => return app.press_record_overwrite(),
        KeyCode::Char(c @ '1'..='9') => return app.select_track(c as usize - '1' as usize),
        KeyCode::Char(' ') => return app.toggle_play(),
        KeyCode::Enter => {
            let from = app.notes().get(app.selected).map(|n| n.note.start_beats);
            return app.play_from_beat(from);
        }
        KeyCode::Char('z') => return app.fit_view(),
        KeyCode::Home => return app.scrub_to(0.0),
        KeyCode::Char('{') => return app.scrub(-4.0),
        KeyCode::Char('}') => return app.scrub(4.0),
        KeyCode::Char('i') => return app.open_picker(),
        KeyCode::Char('I') => return app.open_picker_for_track(),
        KeyCode::Char('t') => return app.toggle_lower(),
        KeyCode::Char('u') => return app.undo(),
        KeyCode::Tab => return app.cycle_focus(),
        KeyCode::BackTab => {
            app.cycle_focus();
            return app.cycle_focus();
        }
        KeyCode::Esc => {
            app.stop();
            if app.embedded.is_some() {
                app.quit = true;
            }
            return;
        }
        _ => {}
    }
    if app.focus == Focus::Mixer {
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => app.mixer_select(-1),
            KeyCode::Char('l') | KeyCode::Right => app.mixer_select(1),
            KeyCode::Char('k') | KeyCode::Up => app.mixer_gain(1.0),
            KeyCode::Char('j') | KeyCode::Down => app.mixer_gain(-1.0),
            KeyCode::Char('[') => app.mixer_pan(-0.1),
            KeyCode::Char(']') => app.mixer_pan(0.1),
            KeyCode::Char('m') => app.mixer_mute(),
            KeyCode::Char('s') => app.mixer_solo(),
            KeyCode::Char('x') | KeyCode::Delete if app.mixer_sel < app.tracks.len() => {
                app.delete_track()
            }
            _ => {}
        }
        return;
    }
    match key.code {
        KeyCode::Char('[') => app.scrub(-1.0),
        KeyCode::Char(']') => app.scrub(1.0),
        // Vim motions: h/l or b/n step through notes, 0/^ and $ (or g/G)
        // jump to the first and last note. The playhead follows.
        KeyCode::Char('h') | KeyCode::Char('b') | KeyCode::Left => app.select(-1),
        KeyCode::Char('l') | KeyCode::Char('n') | KeyCode::Right => app.select(1),
        KeyCode::Char('0') | KeyCode::Char('^') | KeyCode::Char('g') => app.jump_to_first(),
        KeyCode::Char('$') => app.jump_to_last(),
        KeyCode::Char('a') => app.add_note(),
        KeyCode::Char('G') | KeyCode::End => app.jump_to_last(),
        KeyCode::Char('k') | KeyCode::Up => app.transpose(1),
        KeyCode::Char('j') | KeyCode::Down => app.transpose(-1),
        KeyCode::Char('K') => app.transpose(12),
        KeyCode::Char('J') => app.transpose(-12),
        KeyCode::Char('>') | KeyCode::Char('.') => app.change_length(1),
        KeyCode::Char('<') | KeyCode::Char(',') => app.change_length(-1),
        KeyCode::Char('L') => app.shift_time(1),
        KeyCode::Char('H') => app.shift_time(-1),
        KeyCode::Char('x') | KeyCode::Delete => app.delete(),
        KeyCode::Char('d') => app.cut(),
        KeyCode::Char('v') => app.enter_visual(),
        KeyCode::Char('V') => app.select_all(),
        KeyCode::Char('y') => app.yank(),
        KeyCode::Char('p') => app.paste(),
        KeyCode::Char('M') => app.join_next(),
        KeyCode::Char('D') => app.delete_track(),
        KeyCode::Char('+') | KeyCode::Char('=') => {
            // Zoom in: back to one semitone per row first, then in time.
            if app.view.pitch_step > 1 {
                app.view.pitch_step = 1;
            } else {
                app.view.zoom = (app.view.zoom / 2.0).max(1.0 / 32.0);
            }
        }
        KeyCode::Char('-') | KeyCode::Char('_') => app.view.zoom = (app.view.zoom * 2.0).min(2.0),
        _ => {}
    }
}

/// Vim visual mode: motions extend the selection, edits apply to all of it.
fn visual(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('v') => {
            app.exit_visual();
            app.info("");
        }
        KeyCode::Char('V') => app.select_all(),
        KeyCode::Char('h') | KeyCode::Char('b') | KeyCode::Left => app.select(-1),
        KeyCode::Char('l') | KeyCode::Char('n') | KeyCode::Right => app.select(1),
        KeyCode::Char('0') | KeyCode::Char('^') | KeyCode::Char('g') => app.jump_to_first(),
        KeyCode::Char('$') | KeyCode::Char('G') | KeyCode::End => app.jump_to_last(),
        KeyCode::Char('k') | KeyCode::Up => app.transpose(1),
        KeyCode::Char('j') | KeyCode::Down => app.transpose(-1),
        KeyCode::Char('K') => app.transpose(12),
        KeyCode::Char('J') => app.transpose(-12),
        KeyCode::Char('>') | KeyCode::Char('.') => app.change_length(1),
        KeyCode::Char('<') | KeyCode::Char(',') => app.change_length(-1),
        KeyCode::Char('L') => app.shift_time(1),
        KeyCode::Char('H') => app.shift_time(-1),
        KeyCode::Char('y') => app.yank(),
        KeyCode::Char('d') => app.cut(),
        KeyCode::Char('x') | KeyCode::Delete => app.delete(),
        KeyCode::Char('p') => app.paste(),
        KeyCode::Char(' ') => app.toggle_play(),
        KeyCode::Char('u') => {
            app.exit_visual();
            app.undo();
        }
        _ => {}
    }
}

fn command(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => app.run_command_line(),
        KeyCode::Esc => app.mode = Mode::Normal,
        KeyCode::Backspace => {
            if app.command.pop().is_none() {
                app.mode = Mode::Normal;
            }
        }
        KeyCode::Up => app.command_history(true),
        KeyCode::Down => app.command_history(false),
        KeyCode::Char(c) => app.command.push(c),
        _ => {}
    }
}

fn picker(app: &mut App, key: KeyEvent, ctrl: bool) {
    match key.code {
        KeyCode::Enter => app.picker_choose(),
        KeyCode::Esc => app.mode = Mode::Normal,
        KeyCode::Up => app.picker_move(-1),
        KeyCode::Down => app.picker_move(1),
        KeyCode::Char('p') if ctrl => app.picker_move(-1),
        KeyCode::Char('n') if ctrl => app.picker_move(1),
        KeyCode::PageUp => app.picker_move(-10),
        KeyCode::PageDown => app.picker_move(10),
        KeyCode::Backspace => {
            app.picker_query.pop();
            app.refresh_picker();
        }
        KeyCode::Char(c) => {
            app.picker_query.push(c);
            app.refresh_picker();
        }
        _ => {}
    }
}

pub fn on_mouse(app: &mut App, ev: MouseEvent) {
    if !app.mouse || app.mode != Mode::Normal {
        return;
    }
    let pos = Position::new(ev.column, ev.row);
    let grid = app.areas.roll_grid;
    if matches!(
        ev.kind,
        MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight
    ) {
        // Manual scrolling: stop auto-following the selection.
        app.view_follow = false;
    }
    match ev.kind {
        MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left)
            if app.areas.ruler.contains(pos) =>
        {
            let beat =
                app.view.left_beat + f64::from(ev.column - app.areas.ruler.x) * app.view.zoom;
            app.scrub_to((beat / app.grid_beats()).round() * app.grid_beats());
        }
        MouseEventKind::Down(MouseButton::Left)
            if app
                .areas
                .tabs
                .iter()
                .flatten()
                .any(|(r, _)| r.contains(pos)) =>
        {
            if let Some((_, t)) = app
                .areas
                .tabs
                .iter()
                .flatten()
                .find(|(r, _)| r.contains(pos))
            {
                app.select_track(*t);
            }
        }
        MouseEventKind::Down(MouseButton::Right) if grid.contains(pos) => {
            // Right-click on the roll adds a note there, snapped to the grid.
            let g = app.grid_beats();
            let beat = app.view.left_beat + f64::from(ev.column - grid.x) * app.view.zoom;
            let pitch = i32::from(app.view.top_pitch)
                - i32::from(ev.row - grid.y) * i32::from(app.view.pitch_step.max(1));
            if (0..=127).contains(&pitch) {
                app.add_note_at(pitch as u8, (beat / g).floor() * g, g.max(1.0));
            }
        }
        MouseEventKind::Down(MouseButton::Left) if grid.contains(pos) => {
            app.focus = Focus::Roll;
            let beat = app.view.left_beat + f64::from(ev.column - grid.x) * app.view.zoom;
            let pitch = i32::from(app.view.top_pitch)
                - i32::from(ev.row - grid.y) * i32::from(app.view.pitch_step.max(1));
            // Pick the note under the cursor (on any track, switching to
            // it), else the nearest note in time on the current track.
            let under = |n: &crate::song::EditNote| {
                i32::from(n.note.pitch) == pitch && beat >= n.start() && beat < n.end()
            };
            let other = (0..app.tracks.len())
                .filter(|&t| t != app.cur)
                .find_map(|t| app.tracks[t].notes.iter().position(under).map(|i| (t, i)));
            if app.notes().iter().all(|n| !under(n)) {
                if let Some((t, i)) = other {
                    app.select_track(t);
                    app.selected = i;
                    return;
                }
            }
            let hit = app.notes().iter().position(under);
            let nearest = app
                .notes()
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    let da = (a.1.note.start_beats - beat).abs();
                    let db = (b.1.note.start_beats - beat).abs();
                    da.total_cmp(&db)
                })
                .map(|(i, _)| i);
            if let Some(i) = hit.or(nearest) {
                app.selected = i;
            }
        }
        MouseEventKind::Down(MouseButton::Left) if app.areas.lower.contains(pos) => {
            app.focus = Focus::Tracker
        }
        MouseEventKind::Down(MouseButton::Left) if app.areas.mixer.contains(pos) => {
            app.focus = Focus::Mixer
        }
        MouseEventKind::ScrollUp if grid.contains(pos) => {
            app.view.top_pitch = app
                .view
                .top_pitch
                .saturating_add(2 * app.view.pitch_step)
                .min(127)
        }
        MouseEventKind::ScrollDown if grid.contains(pos) => {
            app.view.top_pitch = app.view.top_pitch.saturating_sub(2 * app.view.pitch_step)
        }
        MouseEventKind::ScrollLeft if grid.contains(pos) => {
            app.view.left_beat = (app.view.left_beat - 1.0).max(0.0)
        }
        MouseEventKind::ScrollRight if grid.contains(pos) => app.view.left_beat += 1.0,
        _ => {}
    }
}
