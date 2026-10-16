use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

pub(crate) fn studio() -> Studio {
    Studio::new(
        fixture::mockup_project(),
        &StudioOptions {
            animations: false,
            mouse: false,
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
