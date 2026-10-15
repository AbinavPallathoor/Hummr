//! Prints the editor screen as text: `cargo run -p vme-tui --example screen -- take.wav [keys] [w] [h]`

fn main() {
    let mut args = std::env::args().skip(1);
    let file = args
        .next()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from);
    let keys = args
        .next()
        .unwrap_or_default()
        .replace("\\n", "\n")
        .replace("\\t", "\t");
    let w = args.next().and_then(|s| s.parse().ok()).unwrap_or(110);
    let h = args.next().and_then(|s| s.parse().ok()).unwrap_or(34);
    print!("{}", vme_tui::render_text(file.as_deref(), &keys, w, h));
}
