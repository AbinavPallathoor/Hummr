//! Regenerates the README screenshots:
//! `cargo run -p hummr-tui --example screenshots -- [take.wav|-] [docs/screenshots]`
//!
//! The timeline screens are always written; the notes-editor screens need a
//! hummed take (pass `-` to skip them).

fn main() {
    let mut args = std::env::args().skip(1);
    let take = args
        .next()
        .filter(|t| t != "-")
        .map(std::path::PathBuf::from);
    let out = std::path::PathBuf::from(args.next().unwrap_or_else(|| "docs/screenshots".into()));
    let mut written =
        hummr_tui::screenshot::write_timeline(&out, 118, 36).expect("render timeline screenshots");
    if let Some(take) = take {
        written.extend(
            hummr_tui::screenshot::write_all(&take, &out, 118, 36)
                .expect("render notes-editor screenshots"),
        );
    }
    for path in written {
        println!("wrote {path}");
    }
}
