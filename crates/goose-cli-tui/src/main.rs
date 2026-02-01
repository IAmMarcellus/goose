use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use goose::session::{SessionManager, SessionType};
use goose_cli::session::{build_session, set_output_sink, SessionBuilderConfig};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, IsTerminal};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

use goose_cli_tui::app::{layout_chunks, render_content, render_input, render_thinking};
use goose_cli_tui::sink::TuiSink;
use goose_cli_tui::state::TuiState;
use goose_cli_tui::terminal;

fn run_tui_loop(state: Arc<Mutex<TuiState>>, tx: mpsc::Sender<Option<String>>) -> Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let (content_area, thinking_area, input_area) = layout_chunks(area);
            let content = render_content(state.as_ref(), content_area);
            let thinking = render_thinking(state.as_ref());
            let input = render_input(state.as_ref());
            frame.render_widget(content, content_area);
            frame.render_widget(thinking, thinking_area);
            frame.render_widget(input, input_area);
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let _ = tx.send(None);
                        break;
                    }
                    KeyCode::Enter => {
                        let msg = {
                            let mut s = state.lock().unwrap();
                            let msg = s.input_buffer.clone();
                            if msg.is_empty() {
                                continue;
                            }
                            s.input_buffer.clear();
                            msg
                        };
                        let _ = tx.send(Some(msg));
                    }
                    KeyCode::Char(c) => {
                        state.lock().unwrap().input_buffer.push(c);
                    }
                    KeyCode::Backspace => {
                        state.lock().unwrap().input_buffer.pop();
                    }
                    KeyCode::Up => {
                        let mut s = state.lock().unwrap();
                        s.scroll_offset = s.scroll_offset.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        let mut s = state.lock().unwrap();
                        s.scroll_offset = s.scroll_offset.saturating_add(1);
                    }
                    _ => {}
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    Ok(())
}

fn run_session_thread(
    state: Arc<Mutex<TuiState>>,
    rx: mpsc::Receiver<Option<String>>,
) -> Result<()> {
    let rt = Runtime::new()?;

    let mut session = rt.block_on(async {
        let session_manager = SessionManager::instance();
        let cwd = std::env::current_dir()?;
        let session = session_manager
            .create_session(cwd, "CLI Session".to_string(), SessionType::User)
            .await?;
        let config = SessionBuilderConfig {
            session_id: Some(session.id),
            interactive: true,
            ..Default::default()
        };
        Ok::<_, anyhow::Error>(build_session(config).await)
    })?;

    let sink = TuiSink::new(state.clone());
    set_output_sink(Some(Box::new(sink)));

    while let Ok(msg_opt) = rx.recv() {
        match msg_opt {
            None => break,
            Some(msg) => {
                let cancel = CancellationToken::new();
                let _ = rt.block_on(session.run_one_turn(&msg, cancel));
            }
        }
    }

    set_output_sink(None);
    Ok(())
}

fn main() -> Result<()> {
    goose_cli::logging::setup_logging(None, None)
        .unwrap_or_else(|e| eprintln!("Warning: Failed to initialize logging: {}", e));

    if !std::io::stdout().is_terminal() {
        anyhow::bail!("goose-tui requires a TTY. Run from a terminal.");
    }

    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = terminal::restore();
        default_panic(info);
    }));

    terminal::init()?;

    let state = Arc::new(Mutex::new(TuiState::new()));
    let (tx, rx) = mpsc::channel();

    let state_clone = state.clone();
    let session_handle = thread::spawn(move || {
        if let Err(e) = run_session_thread(state_clone, rx) {
            eprintln!("Session error: {}", e);
        }
    });

    let result = run_tui_loop(state, tx);

    let _ = terminal::restore();
    let _ = session_handle.join();

    result
}
