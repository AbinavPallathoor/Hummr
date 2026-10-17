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

fn inspected(s: &Studio) -> Clip {
    s.song()
        .clip(s.inspector.as_ref().unwrap().clip)
        .unwrap()
        .clone()
}

#[test]
fn enter_opens_the_inspector_and_esc_closes_it() {
    let mut s = studio();
    s.playhead = 40.0;
    press(&mut s, "<cr>");
    assert_eq!(s.mode, Mode::Normal);
    assert!(
        s.message
            .clone()
            .unwrap()
            .0
            .contains("No clip under the playhead")
    );
    s.playhead = 0.0;
    press(&mut s, "<cr>");
    assert_eq!(s.mode, Mode::Inspector);
    assert_eq!(inspected(&s).name, "beatbox");
    let text = screen(&mut s, 100, 30);
    assert!(text.contains("Turn into"), "{text}");
    assert!(text.contains("Drums"), "{text}");
    press(&mut s, "<tab>");
    assert_eq!(
        s.inspector.as_ref().unwrap().panel,
        inspector::Panel::Effects
    );
    press(&mut s, "3");
    assert_eq!(s.inspector.as_ref().unwrap().panel, inspector::Panel::Fix);
    press(&mut s, "<esc>");
    assert_eq!(s.mode, Mode::Normal);
    assert!(s.inspector.is_none());
}

#[test]
fn inspector_100x30() {
    let mut s = studio();
    press(&mut s, "<cr>2jj");
    insta::assert_snapshot!(screen(&mut s, 100, 30));
}

#[test]
fn slider_steps_are_one_undo_step() {
    let mut s = studio();
    press(&mut s, "<cr>2jjj"); // Effects → Reverb
    assert_eq!(
        s.inspector.as_ref().unwrap().control(),
        inspector::Ctl::Reverb
    );
    press(&mut s, "llll");
    assert!((inspected(&s).fx.reverb - 0.4).abs() < 1e-9);
    press(&mut s, "h");
    assert!((inspected(&s).fx.reverb - 0.3).abs() < 1e-9);
    // Another control is its own step.
    press(&mut s, "jll"); // Echo
    assert!((inspected(&s).fx.echo - 0.2).abs() < 1e-9);
    press(&mut s, "u");
    assert_eq!(inspected(&s).fx.echo, 0.0);
    assert!((inspected(&s).fx.reverb - 0.3).abs() < 1e-9);
    press(&mut s, "u");
    assert_eq!(inspected(&s).fx.reverb, 0.0);
    assert_eq!(s.mode, Mode::Inspector);
    press(&mut s, "<c-r>");
    assert!((inspected(&s).fx.reverb - 0.3).abs() < 1e-9);
    // Presets cycle and set an amount.
    press(&mut s, "kkkkl"); // Preset
    let fx = inspected(&s).fx;
    assert!(fx.preset.is_some());
    assert!(fx.amount > 0.0);
}

#[test]
fn turn_into_drums_and_back() {
    let mut s = studio();
    press(&mut s, "<cr>jjj<cr>"); // Drums
    match &inspected(&s).kind {
        ClipKind::Notes { voice, notes, .. } => {
            assert_eq!(*voice, Voice::Drums);
            assert!(notes.len() >= 8, "{} hits", notes.len());
        }
        k => panic!("not drums: {k:?}"),
    }
    let text = screen(&mut s, 100, 30);
    assert!(text.contains("● now"), "{text}");
    press(&mut s, "j<cr>"); // None
    assert_eq!(inspected(&s).kind, ClipKind::Audio);
    press(&mut s, "uu");
    assert_eq!(inspected(&s).kind, ClipKind::Audio);
    // Notes on a chosen instrument: h/l picks it, Enter applies it.
    press(&mut s, "<esc>");
    s.row = 3; // hum take 2 (audio)
    s.playhead = 11.0;
    press(&mut s, "<cr>jl<cr>");
    match &inspected(&s).kind {
        ClipKind::Notes {
            voice: Voice::Instrument { program },
            ..
        } => assert_eq!(*program, inspector::INSTRUMENTS[1]),
        k => panic!("not notes: {k:?}"),
    }
}

#[test]
fn fix_panel_cleans_and_matches_tempo() {
    let mut s = studio();
    s.edit("tempo", |song| song.bpm = 120.0);
    press(&mut s, "<cr>3ll"); // Clean noise
    assert!((inspected(&s).clean - 0.2).abs() < 1e-9);
    press(&mut s, "jl"); // noise sample from 0.1 s
    assert_eq!(inspected(&s).noise_region, Some([0.1, 0.3]));
    press(&mut s, "h");
    assert_eq!(inspected(&s).noise_region, None);
    // Match tempo: the beatbox loop was recorded at 96 BPM.
    press(&mut s, "jj<cr>");
    let est = s.inspector.as_ref().unwrap().estimate.clone().unwrap();
    assert!((est.bpm - 96.0).abs() < 1.0, "heard {}", est.bpm);
    let clip = inspected(&s);
    assert!(
        matches!(clip.kind, ClipKind::Sliced { .. })
            || clip.speed != 1.0
            || clip.stretch != hummr_core::project::Stretch::Keep,
        "{clip:?}"
    );
    let end = clip.end_beat(120.0);
    assert!(
        (end - 8.0).abs() < 0.05,
        "8 beats long at 120 BPM, got {end}"
    );
}

#[test]
fn snap_to_key_moves_notes_into_the_key() {
    let mut s = studio();
    s.row = 2;
    s.playhead = 4.0; // hum → piano
    let id = s.selected_clip_id().unwrap();
    s.edit("off-key", |song| {
        if let Some(ClipKind::Notes { notes, .. }) = song.clip_mut(id).map(|c| &mut c.kind) {
            notes[0].pitch = 61; // C# is not in A minor
        }
    });
    press(&mut s, "<cr>3jjjjj"); // Snap to key
    assert_eq!(
        s.inspector.as_ref().unwrap().control(),
        inspector::Ctl::SnapKey
    );
    press(&mut s, "<cr>");
    assert!(s.message.clone().unwrap().0.contains("Moved 1 note"));
    let ClipKind::Notes { notes, .. } = &inspected(&s).kind else {
        panic!()
    };
    assert!(notes[0].pitch == 60 || notes[0].pitch == 62);
}

fn type_command(s: &mut Studio, cmd: &str) {
    press(s, &format!(":{cmd}<cr>"));
}

#[test]
fn tempo_commands_and_nudges() {
    let mut s = studio();
    type_command(&mut s, "bpm 120");
    assert_eq!(s.song().bpm, 120.0);
    type_command(&mut s, "bpm half");
    assert_eq!(s.song().bpm, 60.0);
    type_command(&mut s, "bpm double");
    assert_eq!(s.song().bpm, 120.0);
    type_command(&mut s, "bpm fast");
    assert_eq!(s.message.clone().unwrap().1, Level::Error);
    type_command(&mut s, "bpm 1000");
    assert_eq!(s.song().bpm, 400.0);
    assert!(s.message.clone().unwrap().0.contains("20 to 400"));
    type_command(&mut s, "bpm 100");
    press(&mut s, ">>><");
    assert_eq!(s.song().bpm, 102.0);
    // The nudges are one undo step.
    press(&mut s, "u");
    assert_eq!(s.song().bpm, 100.0);
    assert!(screen(&mut s, 100, 30).contains("♩ 100 BPM"));
}

#[test]
fn tap_tempo_uses_the_median_gap() {
    let mut s = studio();
    let gaps = [0.0, 0.5, 0.51, 0.49, 0.9, 0.5]; // one late tap
    let mut t = 10.0;
    for (i, g) in gaps.iter().enumerate() {
        t += g;
        s.fake_now = Some(t);
        press(&mut s, "T");
        if i < 2 {
            assert_eq!(s.song().bpm, 96.0, "needs three taps");
        }
    }
    assert_eq!(s.song().bpm, 120.0);
    // A long pause starts a new count; all taps undo as one step.
    s.fake_now = Some(t + 5.0);
    press(&mut s, "T");
    assert_eq!(s.taps.len(), 1);
    press(&mut s, "u");
    assert_eq!(s.song().bpm, 96.0);
}

#[test]
fn time_key_and_snap_commands() {
    let mut s = studio();
    type_command(&mut s, "time 3/4");
    assert_eq!(s.song().time_sig, hummr_core::project::TimeSig::THREE_FOUR);
    assert_eq!(s.song().bar_beats(), 3.0);
    press(&mut s, "}");
    assert_eq!(s.playhead, 3.0);
    type_command(&mut s, "time 5");
    assert_eq!(s.message.clone().unwrap().1, Level::Error);
    type_command(&mut s, "key D major");
    assert_eq!(s.song().key.unwrap().to_string(), "D major");
    type_command(&mut s, "key none");
    assert!(s.song().key.is_none());
    type_command(&mut s, "key H");
    assert_eq!(s.message.clone().unwrap().1, Level::Error);
    type_command(&mut s, "snap 1/8");
    assert_eq!(s.song().snap_beats(), 0.5);
    press(&mut s, "<home>l");
    assert_eq!(s.playhead, 0.5);
    type_command(&mut s, "snap off");
    assert!(s.song().snap.is_none());
    type_command(&mut s, "snap 1/8t");
    assert!((s.song().snap_beats() - 1.0 / 3.0).abs() < 1e-9);
    let text = screen(&mut s, 100, 30);
    assert!(text.contains("3/4 · no key · snap 1/8t"), "{text}");
    press(&mut s, "uuuuuu");
    assert_eq!(s.song().time_sig, hummr_core::project::TimeSig::FOUR_FOUR);
    assert_eq!(s.song().key.unwrap().to_string(), "A minor");
}

#[test]
fn a_tempo_change_re_times_sliced_clips() {
    use hummr_core::onset::detect_onsets;
    use hummr_core::project::render::render_clip;
    let mut s = studio();
    // Cut the beatbox loop at its hits and snap them (Fix → Snap to grid).
    press(&mut s, "<cr>3jjjj<cr><esc>");
    let id = s.selected_clip_id().unwrap();
    let ClipKind::Sliced { slices, .. } = &s.song().clip(id).unwrap().kind else {
        panic!("not sliced")
    };
    let n = slices.len();
    let grid = 0.5;
    for bpm in [96.0, 120.0, 80.0] {
        type_command(&mut s, &format!("bpm {bpm}"));
        let clip = s.song().clip(id).unwrap().clone();
        assert_eq!(clip.start_beat, 0.0);
        assert_eq!(clip.len_beats(bpm), 8.0, "stays two bars long");
        let audio = render_clip(s.song(), &clip, &s.project.sources);
        let step = grid * 60.0 / bpm;
        let onsets = detect_onsets(&audio, hummr_core::project::PROJECT_RATE);
        assert!(onsets.len() >= n / 2, "{} hits at {bpm}", onsets.len());
        // The hits spread over the two bars at this tempo.
        let last = onsets.last().unwrap().time;
        assert!(
            last > 6.0 * 60.0 / bpm && last < 8.0 * 60.0 / bpm,
            "last hit at {last:.3} s at {bpm}"
        );
        let worst = onsets
            .iter()
            .map(|o| {
                let k = (o.time / step).round();
                (o.time - k * step).abs()
            })
            .fold(0.0, f64::max);
        println!("{bpm} BPM: worst hit {:.2} ms off the grid", worst * 1000.0);
        assert!(
            worst < 0.015,
            "a hit is {:.1} ms off the grid at {bpm} BPM",
            worst * 1000.0
        );
    }
}

fn groove(bpm: f64, seed: u64) -> hummr_core::AudioBuffer {
    use hummr_core::project::PROJECT_RATE;
    let (audio, _) = hummr_core::synthetic::drum_groove(bpm, 2, 0, 0.0, 0.005, seed, PROJECT_RATE);
    hummr_core::AudioBuffer::new(audio, PROJECT_RATE)
}

fn empty_studio() -> Studio {
    Studio::new(
        hummr_core::project::Project::new("Empty"),
        &StudioOptions::default(),
    )
}

#[test]
fn the_first_clip_sets_the_song_tempo() {
    let mut s = empty_studio();
    let bpm0 = s.song().bpm;
    s.add_clip_from_audio("loop", groove(104.0, 1), None, None, 0.0);
    assert_eq!(s.mode, Mode::Normal);
    assert!((s.song().bpm - 104.0).abs() <= 1.0, "{}", s.song().bpm);
    let (msg, level) = s.message.clone().unwrap();
    assert!(msg.contains("Song tempo set to 104 BPM"), "{msg}");
    assert_eq!(level, Level::Info);
    // One undo step for the clip, one for the tempo.
    press(&mut s, "u");
    assert_eq!(s.song().bpm, bpm0);
}

#[test]
fn an_unsure_first_tempo_says_so() {
    use hummr_core::project::PROJECT_RATE;
    use hummr_core::synthetic::{Drum, beatbox};
    // Hits on only half of the beats at 100 BPM, so the beat is a guess.
    let spb = 0.6;
    let hits: Vec<(f64, Drum)> = [0, 1, 4, 6, 8, 9, 12, 15, 16, 17, 20, 22, 24, 25, 28, 31]
        .iter()
        .map(|&b| {
            (
                f64::from(b) * spb + 0.05,
                if b % 4 == 0 { Drum::Kick } else { Drum::Snare },
            )
        })
        .collect();
    let audio = beatbox(&hits, 7, PROJECT_RATE);
    let mut s = empty_studio();
    let id = s.add_clip_from_audio(
        "sparse",
        hummr_core::AudioBuffer::new(audio, PROJECT_RATE),
        None,
        None,
        0.0,
    );
    let clip = s.song().clip(id).unwrap().clone();
    let est =
        hummr_core::project::ops::analyze(&clip, s.project.sources.get(&clip.source).unwrap());
    println!(
        "sparse hits: {} (confidence {:.2})",
        est.describe(),
        est.confidence
    );
    assert!(est.has_beat() && !est.is_sure(), "{}", est.describe());
    let (msg, level) = s.message.clone().unwrap();
    assert!(msg.contains("not sure"), "{msg}");
    assert_eq!(level, Level::Warn);
}

#[test]
fn a_sample_at_another_tempo_asks_what_to_do() {
    let mut s = studio(); // 96 BPM
    s.playhead = 20.0;
    let id = s.add_clip_from_audio("loop 120", groove(120.0, 2), None, Some(0), 20.0);
    let Mode::TempoPrompt(p) = s.mode.clone() else {
        panic!("no prompt: {:?}", s.mode)
    };
    assert!((p.bpm - 120.0).abs() < 1.0);
    assert!(p.rhythmic);
    let text = screen(&mut s, 100, 30);
    assert!(text.contains("sounds like 120 BPM (sure)"), "{text}");
    assert!(text.contains("Change song to 120"), "{text}");
    // Half/double switch.
    press(&mut s, "h");
    assert!(screen(&mut s, 100, 30).contains("Change song to 60"));
    press(&mut s, "d");
    // Match to song: cut and snapped to 96 BPM.
    press(&mut s, "<cr>");
    assert_eq!(s.mode, Mode::Normal);
    assert_eq!(s.song().bpm, 96.0);
    let clip = s.song().clip(id).unwrap().clone();
    let ClipKind::Sliced { grid, strength, .. } = clip.kind else {
        panic!("not sliced: {:?}", clip.kind)
    };
    assert_eq!((grid, strength), (0.5, 1.0));
    // Two bars of hits (plus the last hit's ring-out, rounded up to a beat).
    let len = clip.len_beats(96.0);
    assert!((8.0..=10.0).contains(&len), "{len} beats");
    press(&mut s, "u");
    assert_eq!(s.song().clip(id).unwrap().kind, ClipKind::Audio);

    // Change the song instead.
    let id = s.add_clip_from_audio("loop 120 b", groove(120.0, 3), None, Some(0), 40.0);
    press(&mut s, "j<cr>");
    assert!((s.song().bpm - 120.0).abs() < 1.0);
    assert_eq!(s.song().clip(id).unwrap().kind, ClipKind::Audio);

    // Esc leaves it as is.
    let id = s.add_clip_from_audio("loop 150", groove(150.0, 4), None, Some(0), 60.0);
    assert!(matches!(s.mode, Mode::TempoPrompt(_)));
    let before = s.song().clone();
    press(&mut s, "<esc>");
    assert_eq!(s.mode, Mode::Normal);
    assert_eq!(*s.song(), before);
    assert_eq!(s.song().clip(id).unwrap().kind, ClipKind::Audio);
}

#[test]
fn half_or_same_tempo_does_not_ask() {
    let mut s = studio(); // 96 BPM
    s.add_clip_from_audio("same", groove(97.0, 5), None, Some(0), 20.0);
    assert_eq!(s.mode, Mode::Normal);
    s.add_clip_from_audio("double", groove(192.0, 6), None, Some(0), 40.0);
    assert_eq!(s.mode, Mode::Normal, "{:?}", s.message);
    // A one-shot has no tempo.
    press(&mut s, "i<cr>");
    assert_eq!(s.mode, Mode::Normal);
}

#[test]
fn mixer_strips_change_volume_pan_mute_and_solo() {
    let mut s = studio();
    press(&mut s, "M");
    assert!(s.show_mixer);
    assert_eq!(s.mode, Mode::Mixer);
    press(&mut s, "l"); // Bass
    assert_eq!(s.current_track(), Some(1));
    press(&mut s, "jjj]]]");
    assert_eq!(s.song().tracks[1].volume_db, -3.0);
    assert!((s.song().tracks[1].pan - 0.3).abs() < 1e-9);
    press(&mut s, "u");
    assert_eq!(s.song().tracks[1].pan, 0.0);
    assert_eq!(s.song().tracks[1].volume_db, -3.0, "pan undone in one step");
    press(&mut s, "0");
    assert_eq!(s.song().tracks[1].volume_db, 0.0);
    for _ in 0..80 {
        press(&mut s, "j");
    }
    assert_eq!(s.song().tracks[1].volume_db, mixer::MIN_DB);
    press(&mut s, "mlS");
    assert!(s.song().tracks[1].mute);
    assert!(s.song().tracks[2].solo);
    // Esc gives the keys back to the timeline; the strips stay.
    press(&mut s, "<esc>");
    assert_eq!(s.mode, Mode::Normal);
    assert!(s.show_mixer);
    press(&mut s, "M");
    assert!(!s.show_mixer);
}

#[test]
fn meters_show_levels_and_fall() {
    assert_eq!(mixer::meter_cells(0.0, 11), 0);
    assert_eq!(mixer::meter_cells(1.0, 11), 11);
    assert_eq!(mixer::meter_cells(0.5, 11), 10); // −6 dB
    assert_eq!(mixer::meter_cells(0.004, 11), 0); // below −48 dB
    let mut s = studio();
    s.meters = vec![1.0, 0.5, 0.0, 0.1];
    s.master_meter = 1.0;
    s.tick(std::time::Duration::from_millis(500));
    // 24 dB per second: half a second later, 12 dB lower.
    assert!((hummr_core::dsp::amp_to_db(s.meters[0]) + 12.0).abs() < 0.01);
    assert!(s.meters[2] == 0.0);
}

#[test]
fn mixer_100x30() {
    let mut s = studio();
    s.edit("mix", |song| {
        song.tracks[0].volume_db = -4.0;
        song.tracks[1].pan = -0.4;
        song.tracks[2].mute = true;
        song.tracks[3].solo = true;
    });
    press(&mut s, "Ml");
    s.meters = vec![0.9, 0.25, 0.0, 0.05];
    s.master_meter = 0.95;
    insta::assert_snapshot!(screen(&mut s, 100, 30));
}

#[test]
fn animations_run_only_when_on() {
    let mut s = studio();
    s.animations = true;
    screen(&mut s, 100, 30);
    assert!(!s.effects.is_running());
    // A new clip flashes.
    s.playhead = 20.0;
    press(&mut s, "jjjji<cr>");
    assert!(s.flash.is_some());
    screen(&mut s, 100, 30);
    assert!(s.effects.is_running());
    assert!(s.flash.is_none(), "the flash starts once");
    assert!(s.is_live(), "redraws while animating");
    s.tick(std::time::Duration::from_secs(1));
    screen(&mut s, 100, 30);
    s.tick(std::time::Duration::from_millis(20));
    screen(&mut s, 100, 30);
    assert!(!s.effects.is_running(), "effects end");
    // Opening an overlay fades it in.
    s.playhead = 0.0;
    s.row = 0;
    press(&mut s, "<cr>");
    screen(&mut s, 100, 30);
    assert!(s.effects.is_running());
    // :anim off stops everything and starts nothing.
    press(&mut s, "<esc>:anim off<cr>");
    screen(&mut s, 100, 30);
    assert!(!s.effects.is_running());
    press(&mut s, "<cr>");
    screen(&mut s, 100, 30);
    assert!(!s.effects.is_running());
    s.playhead = 30.0;
    press(&mut s, "<esc>i<cr>");
    assert!(s.flash.is_none());
}

#[test]
fn the_view_follows_the_playhead_smoothly_when_animated() {
    for animated in [true, false] {
        let mut s = studio();
        s.animations = animated;
        s.edit("long", |song| {
            let c = song.tracks[0].clips[0].clone();
            for k in 1..8 {
                let mut c = c.clone();
                c.start_beat = 16.0 * f64::from(k);
                song.insert_clip(0, c);
            }
        });
        screen(&mut s, 100, 30);
        let cols = f64::from(s.lane_cols);
        press(&mut s, " ");
        let mut cols_seen = Vec::new();
        for _ in 0..600 {
            s.tick(std::time::Duration::from_millis(50));
            screen(&mut s, 100, 30);
            cols_seen.push(s.view.beat_to_col(s.playhead));
        }
        assert!(s.view.start_beat > 0.0, "the view moved");
        let late = &cols_seen[200..];
        if animated {
            // Once a third of the way in, the playhead stays there.
            for c in late {
                assert!((c - (cols / 3.0).floor()).abs() <= 1.0, "col {c} of {cols}");
            }
        } else {
            // Paging: the playhead sweeps across the view.
            let spread = late.iter().cloned().fold(f64::MIN, f64::max)
                - late.iter().cloned().fold(f64::MAX, f64::min);
            assert!(spread > cols / 2.0, "spread {spread}");
        }
    }
}

fn clip_notes(s: &Studio, id: u64) -> Vec<hummr_core::project::Note> {
    match &s.song().clip(id).unwrap().kind {
        ClipKind::Notes { notes, .. } => notes.clone(),
        k => panic!("not notes: {k:?}"),
    }
}

#[test]
fn e_edits_a_notes_clip_in_the_notes_editor_and_esc_comes_back() {
    let mut s = studio();
    // A recording can't be edited as notes.
    press(&mut s, "e");
    assert_eq!(s.screen, Screen::Timeline);
    assert!(
        s.message
            .clone()
            .unwrap()
            .0
            .contains("turn it into notes first")
    );
    press(&mut s, "j"); // mmm → bass
    let id = s.selected_clip_id().unwrap();
    let before = clip_notes(&s, id);
    assert_eq!(before.len(), 3);
    press(&mut s, "e");
    assert_eq!(s.screen, Screen::Notes);
    let text = screen(&mut s, 100, 30);
    assert!(text.contains("Editing the notes of \"mmm\""), "{text}");
    // Old editor keys: transpose the first note up two, delete the next.
    press(&mut s, "0kklx");
    press(&mut s, "<esc>");
    assert_eq!(s.screen, Screen::Timeline);
    assert!(s.notes_editor.is_none());
    let after = clip_notes(&s, id);
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].pitch, before[0].pitch + 2);
    assert_eq!(after[1], before[2]);
    assert!(s.message.clone().unwrap().0.contains("2 notes"));
    assert!(screen(&mut s, 100, 30).contains("Beatbox Song"));
    // One undo step brings the notes back.
    press(&mut s, "u");
    assert_eq!(clip_notes(&s, id), before);
    // :q also comes back, and :w doesn't write a MIDI file.
    press(&mut s, "e:w<cr>");
    assert_eq!(s.screen, Screen::Notes);
    press(&mut s, ":q<cr>");
    assert_eq!(s.screen, Screen::Timeline);
    assert_eq!(clip_notes(&s, id), before);
}
