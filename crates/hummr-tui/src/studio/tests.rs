use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Types `keys` into the timeline. `<name>` spells special keys: <esc>,
/// <cr>, <home>, <end>, <c-r>, <bs>, <tab>, <up>, <down>, <left>, <right>.
pub(crate) fn press(s: &mut Studio, keys: &str) {
    let mut chars = keys.chars().peekable();
    while let Some(c) = chars.next() {
        let (code, mods) = if c == '<' && chars.peek().is_some_and(|n| n.is_ascii_alphabetic()) {
            let mut name = String::new();
            for n in chars.by_ref() {
                if n == '>' {
                    break;
                }
                name.push(n);
            }
            match name.as_str() {
                "esc" => (KeyCode::Esc, KeyModifiers::NONE),
                "cr" => (KeyCode::Enter, KeyModifiers::NONE),
                "home" => (KeyCode::Home, KeyModifiers::NONE),
                "end" => (KeyCode::End, KeyModifiers::NONE),
                "bs" => (KeyCode::Backspace, KeyModifiers::NONE),
                "tab" => (KeyCode::Tab, KeyModifiers::NONE),
                "up" => (KeyCode::Up, KeyModifiers::NONE),
                "down" => (KeyCode::Down, KeyModifiers::NONE),
                "left" => (KeyCode::Left, KeyModifiers::NONE),
                "right" => (KeyCode::Right, KeyModifiers::NONE),
                "c-r" => (KeyCode::Char('r'), KeyModifiers::CONTROL),
                "c-c" => (KeyCode::Char('c'), KeyModifiers::CONTROL),
                other => panic!("unknown key <{other}>"),
            }
        } else {
            (KeyCode::Char(c), KeyModifiers::NONE)
        };
        keys::on_key(s, KeyEvent::new(code, mods));
        s.tick(std::time::Duration::ZERO);
    }
}

pub(crate) fn studio() -> Studio {
    Studio::new(
        fixture::mockup_project(),
        &StudioOptions {
            animations: false,
            mouse: false,
            audio: false,
        },
    )
}

pub(crate) fn screen(s: &mut Studio, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| draw::draw(f, s)).unwrap();
    let buf = term.backend().buffer();
    let mut out = String::new();
    for y in 0..h {
        let line: String = (0..w).map(|x| buf[(x, y)].symbol()).collect();
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

#[test]
fn rows_include_overlap_lanes() {
    let s = studio();
    let rows = s.rows();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[3], Row { track: 2, lane: 1 });
}

#[test]
fn selection_is_the_clip_under_the_playhead() {
    let mut s = studio();
    s.row = 2;
    s.playhead = 6.0;
    assert_eq!(s.selected_clip().unwrap().name, "hum");
    s.playhead = 3.0;
    assert!(s.selected_clip().is_none());
    s.row = 3;
    s.playhead = 12.0;
    assert_eq!(s.selected_clip().unwrap().name, "hum take 2");
}

#[test]
fn labels() {
    assert_eq!(short_instrument(0), "piano");
    assert_eq!(short_instrument(33), "bass");
    let s = studio();
    let bass = &s.song().tracks[1].clips[0];
    assert_eq!(clip_label(bass), "mmm → bass");
}

fn snap(w: u16, h: u16) -> String {
    let mut s = studio();
    s.row = 2;
    s.playhead = 6.0;
    screen(&mut s, w, h)
}

#[test]
fn timeline_80x24() {
    insta::assert_snapshot!(snap(80, 24));
}

#[test]
fn timeline_100x30() {
    insta::assert_snapshot!(snap(100, 30));
}

#[test]
fn timeline_120x40() {
    insta::assert_snapshot!(snap(120, 40));
}

#[test]
fn too_small() {
    let mut s = studio();
    let text = screen(&mut s, 70, 20);
    assert!(text.contains("Terminal too small"), "{text}");
}

#[test]
fn h_and_l_move_the_playhead_one_snap_step() {
    let mut s = studio();
    press(&mut s, "lll");
    assert_eq!(s.playhead, 3.0);
    press(&mut s, "h");
    assert_eq!(s.playhead, 2.0);
    press(&mut s, "hhhh");
    assert_eq!(s.playhead, 0.0, "stops at the start");
    s.project.song.snap = "1/8".parse().ok();
    press(&mut s, "l");
    assert_eq!(s.playhead, 0.5);
}

#[test]
fn j_and_k_walk_tracks_and_lanes() {
    let mut s = studio();
    press(&mut s, "jj");
    assert_eq!(s.current_row(), Some(Row { track: 2, lane: 0 }));
    press(&mut s, "j");
    assert_eq!(s.current_row(), Some(Row { track: 2, lane: 1 }));
    press(&mut s, "jjjj");
    assert_eq!(
        s.current_row(),
        Some(Row { track: 3, lane: 0 }),
        "stops at the last row"
    );
    press(&mut s, "kkkkkkk");
    assert_eq!(s.row, 0);
}

#[test]
fn bars_home_and_end() {
    let mut s = studio();
    press(&mut s, "}");
    assert_eq!(s.playhead, 4.0);
    press(&mut s, "l}");
    assert_eq!(s.playhead, 8.0);
    press(&mut s, "l{");
    assert_eq!(s.playhead, 8.0);
    press(&mut s, "{");
    assert_eq!(s.playhead, 4.0);
    press(&mut s, "<end>");
    assert_eq!(s.playhead, s.song().end_beats());
    press(&mut s, "<home>");
    assert_eq!(s.playhead, 0.0);
}

#[test]
fn zoom_and_fit() {
    let mut s = studio();
    let _ = screen(&mut s, 80, 24);
    press(&mut s, "+");
    assert_eq!(s.view.cols_per_beat, 8.0);
    press(&mut s, "--");
    assert_eq!(s.view.cols_per_beat, 2.0);
    press(&mut s, "0");
    // 16 beats fit in 67 columns at 4 columns a beat.
    assert_eq!(s.view.cols_per_beat, 4.0);
    assert_eq!(s.view.start_beat, 0.0);
}

#[test]
fn space_plays_and_stops() {
    let mut s = studio();
    press(&mut s, " ");
    assert!(s.playing);
    s.tick(std::time::Duration::from_millis(1_250));
    // Two beats at 96 BPM.
    assert!((s.playhead - 2.0).abs() < 1e-6, "{}", s.playhead);
    press(&mut s, " ");
    assert!(!s.playing);
    assert!((s.playhead - 2.0).abs() < 1e-6);
}

#[test]
fn help_and_command_line() {
    let mut s = studio();
    press(&mut s, "?");
    assert_eq!(s.mode, Mode::Help);
    let text = screen(&mut s, 100, 30);
    assert!(text.contains("Help"), "{text}");
    press(&mut s, "<esc>");
    assert_eq!(s.mode, Mode::Normal);
    press(&mut s, ":nonsense<cr>");
    let (msg, _) = s.message.clone().unwrap();
    assert!(msg.contains("no :nonsense"), "{msg}");
    press(&mut s, ":anim on<cr>");
    assert!(s.animations);
}

#[test]
fn quitting_with_unsaved_changes_warns_once() {
    let mut s = studio();
    s.edit("rename", |song| song.name = "x".into());
    press(&mut s, "q");
    assert!(!s.quit);
    assert!(s.message.clone().unwrap().0.contains("unsaved"));
    press(&mut s, "q");
    assert!(s.quit);
}

#[test]
fn save_and_reopen() {
    let dir = std::env::temp_dir().join(format!("hummr-studio-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = studio();
    let path = dir.join("song.hummr");
    press(&mut s, &format!(":w {}<cr>", path.display()));
    assert!(
        s.message.clone().unwrap().0.starts_with("Saved"),
        "{:?}",
        s.message
    );
    assert!(!s.is_dirty());
    let p = hummr_core::project::Project::open(&path).unwrap();
    assert_eq!(p.song, s.project.song);
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    press(&mut s, ":export out.wav<cr>");
    assert!(dir.join("out.wav").exists(), "{:?}", s.message);
}

fn render(s: &Studio) -> hummr_core::project::render::Stereo {
    hummr_core::project::render::mixdown(
        s.song(),
        &s.project.sources,
        hummr_core::project::render::MixOptions { limiter: false },
    )
}

/// Start beat of the selected clip.
fn sel_start(s: &Studio) -> f64 {
    s.selected_clip().expect("a selected clip").start_beat
}

#[test]
fn shift_h_and_l_move_the_clip_on_the_grid() {
    let mut s = studio();
    press(&mut s, "jjj"); // Lead lane 2: "hum take 2" at beat 10
    s.playhead = 12.0;
    assert_eq!(s.selected_clip().unwrap().name, "hum take 2");
    press(&mut s, "L");
    assert_eq!(sel_start(&s), 11.0);
    assert_eq!(s.playhead, 13.0, "the playhead moves with the clip");
    press(&mut s, "HHH");
    assert_eq!(sel_start(&s), 8.0);
    // An off-grid clip lands on the grid.
    let id = s.selected_clip_id().unwrap();
    s.edit("nudge", |song| song.clip_mut(id).unwrap().start_beat = 8.3);
    s.project.song.snap = "1/8".parse().ok();
    press(&mut s, "L");
    assert_eq!(s.song().clip(id).unwrap().start_beat, 8.5);
    press(&mut s, "H");
    assert_eq!(s.song().clip(id).unwrap().start_beat, 8.0);
}

#[test]
fn overlap_makes_a_second_lane_that_collapses_again() {
    let mut s = studio();
    // Sample track: "vocal chop" at 2..4.4 and "slam" at 13.
    press(&mut s, "jjjj");
    assert_eq!(s.current_track(), Some(3));
    assert_eq!(s.song().lane_count(3), 1);
    s.playhead = 13.0;
    assert_eq!(s.selected_clip().unwrap().name, "slam");
    // Move "slam" back onto "vocal chop".
    press(&mut s, &"H".repeat(10));
    assert_eq!(sel_start(&s), 3.0);
    assert_eq!(s.song().lane_count(3), 2);
    assert_eq!(
        s.current_row(),
        Some(Row { track: 3, lane: 1 }),
        "the selection follows the clip"
    );
    let text = screen(&mut s, 120, 40);
    assert!(text.contains("↳ lane 2"));
    assert!(text.matches("lane 2").count() >= 2, "{text}");
    insta::assert_snapshot!("overlap_lane_120x40", text);
    // Moving it away collapses the lane again.
    press(&mut s, "LLLL");
    assert_eq!(s.song().lane_count(3), 1);
    assert_eq!(s.current_row(), Some(Row { track: 3, lane: 0 }));
    // And undoing the four moves brings the overlap back.
    press(&mut s, "uuuu");
    assert_eq!(s.song().lane_count(3), 2);
}

#[test]
fn shift_j_and_k_move_the_clip_between_tracks() {
    let mut s = studio();
    s.playhead = 3.0;
    press(&mut s, "jjjj");
    assert_eq!(s.selected_clip().unwrap().name, "vocal chop");
    press(&mut s, "K");
    assert_eq!(s.current_track(), Some(2));
    assert_eq!(s.selected_clip().unwrap().name, "vocal chop");
    // It overlaps "hum" (4..12) on Lead, so it got its own lane.
    assert_eq!(s.song().lane_count(2), 2);
    press(&mut s, "J");
    assert_eq!(s.current_track(), Some(3));
    press(&mut s, "J");
    assert!(s.message.clone().unwrap().0.contains("last track"));
}

#[test]
fn split_then_play_equals_the_original() {
    let mut s = studio();
    let before = render(&s);
    s.playhead = 5.0;
    press(&mut s, "s"); // beatbox at 0..8
    assert_eq!(s.song().tracks[0].clips.len(), 3);
    press(&mut s, "jjj");
    s.playhead = 12.5;
    press(&mut s, "s"); // hum take 2
    assert_eq!(s.song().tracks[2].clips.len(), 3);
    let after = render(&s);
    assert_eq!(before.len(), after.len());
    let worst = before
        .left
        .iter()
        .zip(&after.left)
        .chain(before.right.iter().zip(&after.right))
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(worst < 1e-6, "split changed the sound by {worst}");
    // Splitting at the very edge does nothing and says why.
    s.row = 0;
    s.playhead = 8.0;
    press(&mut s, "s");
    assert!(s.message.clone().unwrap().0.contains("edge"));
}

#[test]
fn copy_paste_duplicate_delete_and_undo() {
    let mut s = studio();
    let n0 = s.song().tracks[0].clips.len();
    press(&mut s, "y");
    s.playhead = 16.0;
    press(&mut s, "p");
    assert_eq!(s.song().tracks[0].clips.len(), n0 + 1);
    assert_eq!(sel_start(&s), 16.0);
    assert_eq!(s.selected_clip().unwrap().name, "beatbox");
    press(&mut s, "d");
    assert_eq!(s.song().tracks[0].clips.len(), n0 + 2);
    assert_eq!(
        sel_start(&s),
        24.0,
        "the copy starts where the original ends"
    );
    press(&mut s, "x");
    assert_eq!(s.song().tracks[0].clips.len(), n0 + 1);
    press(&mut s, "uu");
    assert_eq!(s.song().tracks[0].clips.len(), n0 + 1);
    press(&mut s, "u");
    assert_eq!(s.song().tracks[0].clips.len(), n0);
    press(&mut s, "<c-r><c-r>");
    assert_eq!(s.song().tracks[0].clips.len(), n0 + 2);
    // With nothing under the playhead, clip keys explain.
    s.playhead = 40.0;
    press(&mut s, "x");
    assert!(
        s.message
            .clone()
            .unwrap()
            .0
            .contains("No clip under the playhead")
    );
}

#[test]
fn track_keys() {
    let mut s = studio();
    press(&mut s, "m");
    assert!(s.song().tracks[0].mute);
    press(&mut s, "jS");
    assert!(s.song().tracks[1].solo);
    assert!(!s.song().audible(0) && s.song().audible(1));
    press(&mut s, "n");
    assert_eq!(s.song().tracks.len(), 5);
    assert_eq!(s.current_track(), Some(4));
    press(&mut s, ":track name Pads<cr>");
    assert_eq!(s.song().tracks[4].name, "Pads");
    press(&mut s, ":track delete<cr>");
    assert_eq!(s.song().tracks.len(), 4);
    press(&mut s, "u");
    assert_eq!(s.song().tracks[4].name, "Pads");
}

#[test]
fn import_a_starter_sample_at_the_playhead() {
    let mut s = studio();
    s.playhead = 20.0;
    press(&mut s, "jjjji");
    let Mode::Import(p) = &s.mode else {
        panic!("{:?}", s.mode)
    };
    assert_eq!(p.items[0].label(), "kick (built-in)");
    let text = screen(&mut s, 100, 30);
    assert!(text.contains("Import a sample"), "{text}");
    press(&mut s, "jj<cr>"); // clap
    assert_eq!(s.mode, Mode::Normal);
    let c = s.selected_clip().unwrap();
    assert_eq!(c.name, "clap");
    assert_eq!(c.start_beat, 20.0);
    assert_eq!(s.current_track(), Some(3));
}

#[test]
fn a_recorded_take_lands_at_the_playhead_with_the_song_tempo() {
    let mut s = studio();
    s.playhead = 16.0;
    press(&mut s, "j");
    let take = hummr_core::AudioBuffer::new(vec![0.1; 48_000], 48_000);
    let id = s.add_take(take, 1, 16.0);
    let c = s.song().clip(id).unwrap().clone();
    assert_eq!(c.start_beat, 16.0);
    assert_eq!(c.name, "take 1");
    let src = s.song().source(&c.source).unwrap();
    assert_eq!(src.recorded_bpm, Some(96.0));
    assert_eq!(s.selected_clip_id(), Some(id));
}
