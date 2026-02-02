use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use goose::permission::Permission;
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

use goose_cli_tui::app::{
    content_area_with_overlay, input_cursor_position, layout_chunks, overlay_area, render_content,
    render_input, render_overlay, render_thinking,
};
use goose_cli_tui::history::HistoryEvent;
use goose_cli_tui::overlay::{Overlay, THEMES};
use goose_cli_tui::sink::TuiSink;
use goose_cli_tui::state::TuiState;
use goose_cli_tui::terminal;

const SCROLL_PAGE_SIZE: usize = 10;

fn run_tui_loop(state: Arc<Mutex<TuiState>>, tx: mpsc::Sender<Option<String>>) -> Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    loop {
        let overlay_active = state.lock().unwrap().overlay.is_active();
        terminal.draw(|frame| {
            let area = frame.area();
            let main_area = content_area_with_overlay(area, overlay_active);
            let (content_area, thinking_area, input_area) = layout_chunks(main_area);
            let content_widget = render_content(state.as_ref(), content_area);
            let thinking = render_thinking(state.as_ref());
            let input = render_input(state.as_ref());
            frame.render_widget(content_widget, content_area);
            frame.render_widget(thinking, thinking_area);
            frame.render_widget(input, input_area);
            if overlay_active {
                if let Some(overlay_widget) = render_overlay(state.as_ref(), area) {
                    frame.render_widget(overlay_widget, overlay_area(area));
                }
            } else {
                let buffer_len = state.lock().unwrap().input_buffer.chars().count();
                let (cx, cy) = input_cursor_position(input_area, buffer_len);
                frame.set_cursor_position((cx, cy));
            }
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let mut s = state.lock().unwrap();
                    if s.overlay.is_active() {
                        match key.code {
                            KeyCode::Esc => {
                                if let Overlay::Approval { .. } = &s.overlay {
                                    if let Some(ref approval_tx) = s.approval_response_tx {
                                        let _ = approval_tx.send(Permission::Cancel);
                                    }
                                }
                                s.overlay = Overlay::None;
                            }
                            KeyCode::Up => {
                                if let Overlay::ThemePicker { selected, .. } = &mut s.overlay {
                                    *selected = selected.saturating_sub(1);
                                }
                            }
                            KeyCode::Down => {
                                if let Overlay::ThemePicker { selected, .. } = &mut s.overlay {
                                    *selected = (*selected + 1).min(THEMES.len().saturating_sub(1));
                                }
                            }
                            KeyCode::Enter => {
                                if let Overlay::ThemePicker { selected, .. } = &mut s.overlay {
                                    let theme = THEMES.get(*selected).copied().unwrap_or(THEMES[0]);
                                    s.theme = theme;
                                    s.overlay = Overlay::None;
                                    drop(s);
                                    goose_cli::session::set_theme(match theme {
                                        goose_cli_tui::content_render::Theme::Dark => {
                                            goose_cli::session::OutputTheme::Dark
                                        }
                                        goose_cli_tui::content_render::Theme::Light => {
                                            goose_cli::session::OutputTheme::Light
                                        }
                                        goose_cli_tui::content_render::Theme::Ansi => {
                                            goose_cli::session::OutputTheme::Ansi
                                        }
                                    });
                                    continue;
                                }
                            }
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                if let Overlay::Approval { .. } = &s.overlay {
                                    if let Some(ref tx) = s.approval_response_tx {
                                        let _ = tx.send(Permission::AllowOnce);
                                    }
                                    s.overlay = Overlay::None;
                                }
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') => {
                                if let Overlay::Approval { .. } = &s.overlay {
                                    if let Some(ref tx) = s.approval_response_tx {
                                        let _ = tx.send(Permission::DenyOnce);
                                    }
                                    s.overlay = Overlay::None;
                                }
                            }
                            _ => {}
                        }
                        drop(s);
                        continue;
                    }
                    drop(s);
                    match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            {
                                let mut s = state.lock().unwrap();
                                if let Overlay::Approval { .. } = &s.overlay {
                                    if let Some(ref approval_tx) = s.approval_response_tx {
                                        let _ = approval_tx.send(Permission::Cancel);
                                    }
                                    s.overlay = Overlay::None;
                                }
                            }
                            let _ = tx.send(None);
                            break;
                        }
                        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            let mut s = state.lock().unwrap();
                            let selected = THEMES.iter().position(|&t| t == s.theme).unwrap_or(0);
                            s.overlay = Overlay::ThemePicker { selected };
                        }
                        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            state.lock().unwrap().overlay = Overlay::Settings;
                        }
                        KeyCode::Enter => {
                            let msg = {
                                let mut s = state.lock().unwrap();
                                let msg = s.input_buffer.clone();
                                if msg.is_empty() {
                                    continue;
                                }
                                s.input_buffer.clear();
                                s.scroll_follows_stream = true;
                                s.history
                                    .apply_event(HistoryEvent::UserMessage { text: msg.clone() });
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
                            s.scroll_follows_stream = false;
                        }
                        KeyCode::Down => {
                            let mut s = state.lock().unwrap();
                            s.scroll_offset = s.scroll_offset.saturating_add(1);
                        }
                        KeyCode::PageUp => {
                            let mut s = state.lock().unwrap();
                            s.scroll_offset = s.scroll_offset.saturating_sub(SCROLL_PAGE_SIZE);
                            s.scroll_follows_stream = false;
                        }
                        KeyCode::PageDown => {
                            let mut s = state.lock().unwrap();
                            s.scroll_offset = s.scroll_offset.saturating_add(SCROLL_PAGE_SIZE);
                        }
                        KeyCode::Home => {
                            let mut s = state.lock().unwrap();
                            s.scroll_offset = 0;
                            s.scroll_follows_stream = false;
                        }
                        KeyCode::End => {
                            let mut s = state.lock().unwrap();
                            s.scroll_offset = usize::MAX;
                            s.scroll_follows_stream = true;
                        }
                        _ => {}
                    }
                }
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
    approval_rx: mpsc::Receiver<Permission>,
) -> Result<()> {
    let rt = Runtime::new()?;

    let sink = TuiSink::new(state.clone(), approval_rx);
    set_output_sink(Some(Box::new(sink)));

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

    goose_cli::session::hide_thinking();

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

    let (approval_tx, approval_rx) = mpsc::channel();
    let mut state_inner = TuiState::new();
    state_inner.approval_response_tx = Some(approval_tx);
    state_inner.theme = match goose_cli::session::get_theme() {
        goose_cli::session::OutputTheme::Dark => goose_cli_tui::content_render::Theme::Dark,
        goose_cli::session::OutputTheme::Light => goose_cli_tui::content_render::Theme::Light,
        goose_cli::session::OutputTheme::Ansi => goose_cli_tui::content_render::Theme::Ansi,
    };
    let state = Arc::new(Mutex::new(state_inner));
    let (tx, rx) = mpsc::channel();

    let state_clone = state.clone();
    let session_handle = thread::spawn(move || {
        if let Err(e) = run_session_thread(state_clone, rx, approval_rx) {
            eprintln!("Session error: {}", e);
        }
    });

    let result = run_tui_loop(state, tx);

    let _ = terminal::restore();
    let _ = session_handle.join();

    result
}
