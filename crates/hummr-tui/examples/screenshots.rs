//! Regenerates the README screenshots:
//! `cargo run -p hummr-tui --example screenshots -- take.wav docs/screenshots`

fn main() {
    let mut args = std::env::args().skip(1);
    let take = std::path::PathBuf::from(args.next().expect("usage: screenshots TAKE.wav OUT_DIR"));
    let out = std::path::PathBuf::from(args.next().unwrap_or_else(|| "docs/screenshots".into()));
    for path in hummr_tui::screenshot::write_all(&take, &out, 118, 36).expect("render screenshots")
    {
        println!("wrote {path}");
    }
}
