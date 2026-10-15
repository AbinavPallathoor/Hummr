//! Terminal editor for Hummr.
//!
//! Layout: piano roll on top, tracker (or "why notes changed") and mixer
//! below, a message line and a hint bar. Editing is vim-like with a `:`
//! command line. All audio work goes through `hummr-core`.

mod app;
mod command;
mod input;
mod smooth;
mod ui;

use std::io::stdout;
use std::time::{Duration, Instant};

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
};
use ratatui::crossterm::execute;

use app::App;
pub use app::Options;

/// Target frame time while something is moving (~60 fps).
const FRAME: Duration = Duration::from_millis(16);
/// Poll interval when idle, to keep CPU use near zero.
const IDLE: Duration = Duration::from_millis(100);

/// Runs the editor until the user quits.
pub fn run(opts: Options) -> anyhow::Result<()> {
    let mut app = App::new(&opts);
    if let Some(file) = &opts.file {
        app.open(file);
    }
    let mut terminal = ratatui::init();
    if app.mouse {
        let _ = execute!(stdout(), EnableMouseCapture);
    }
    let result = run_loop(&mut terminal, &mut app);
    if app.mouse {
        let _ = execute!(stdout(), DisableMouseCapture);
    }
    ratatui::restore();
    result
}

fn run_loop(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    let mut last = Instant::now();
    loop {
        let frame_start = Instant::now();
        let dt = frame_start - last;
        last = frame_start;
        app.tick(dt);
        terminal.draw(|f| ui::draw(f, app, dt))?;
        if app.quit {
            app.stop();
            return Ok(());
        }
        if std::mem::take(&mut app.mouse_changed) {
            let _ = if app.mouse {
                execute!(stdout(), EnableMouseCapture)
            } else {
                execute!(stdout(), DisableMouseCapture)
            };
        }

        let budget = if app.is_live() { FRAME } else { IDLE };
        let mut timeout = budget.saturating_sub(frame_start.elapsed());
        while event::poll(timeout)? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => input::on_key(app, k),
                Event::Mouse(m) => input::on_mouse(app, m),
                _ => {}
            }
            if app.quit {
                break;
            }
            // Drain anything else already queued, then draw.
            timeout = Duration::ZERO;
        }
    }
}

/// Renders the editor with `file` loaded into a plain-text screen of
/// `width`×`height`, after `keys` are typed. For snapshots and debugging.
#[doc(hidden)]
pub fn render_text(file: Option<&std::path::Path>, keys: &str, width: u16, height: u16) -> String {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = App::new(&Options {
        file: None,
        animations: false,
        mouse: false,
    });
    if let Some(f) = file {
        app.open(f);
        while app.busy.is_some() {
            std::thread::sleep(Duration::from_millis(5));
            app.tick(Duration::from_millis(5));
        }
    }
    for c in keys.chars() {
        let code = match c {
            '\n' => KeyCode::Enter,
            '\t' => KeyCode::Tab,
            '\u{1b}' => KeyCode::Esc,
            c => KeyCode::Char(c),
        };
        input::on_key(&mut app, KeyEvent::new(code, KeyModifiers::NONE));
    }
    let mut term = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    term.draw(|f| ui::draw(f, &mut app, Duration::from_millis(16)))
        .expect("draw");
    let buf = term.backend().buffer();
    let mut out = String::new();
    for y in 0..height {
        for x in 0..width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}
