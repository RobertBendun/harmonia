#![allow(clippy::missing_docs_in_private_items)]
//! Playground to test [linky_groups] using terminal interface.
//!
//! It's deprectaed and will be removed in future release.

use crossterm::{
    event::{Event, KeyCode, KeyEvent, KeyModifiers},
    execute, terminal,
};
use futures::StreamExt;
use std::{io::Write, ops::ControlFlow, sync::Arc, time::Duration};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "linky_groups=debug,linky_groups::net=info".into()),
        )
        .with(tracing_subscriber::fmt::Layer::new().event_format(RawTerminalFormatter::default()))
        .init();

    print!("\rq - quit, <space> - start, s - stop, up - incr id, down - decr id\n");
    crossterm::terminal::enable_raw_mode().unwrap();

    let link = Arc::new(rusty_link::AblLink::new(120.0));
    link.enable(true);
    link.enable_start_stop_sync(false); // not nessesary, but we wan't to explicitly disable it
                                        // just to be sure

    let groups = linky_groups::listen(link.clone());

    let mut stderr = std::io::stderr();
    let mut stdout = std::io::stdout();

    let mut keys = crossterm::event::EventStream::new().fuse();
    let mut id = 0_u64;

    loop {
        let timeout = tokio::time::sleep(Duration::from_micros(10));

        tokio::select! {
            event = keys.next() => {
                match event {
                    Some(Ok(event)) => {
                        match on_key_press(event, &mut id) {
                            ControlFlow::Continue(Action::None) => {},
                            ControlFlow::Continue(Action::Start(group_name)) => {
                                groups.start(&group_name).await.unwrap();
                            },
                            ControlFlow::Continue(Action::Stop) => {
                                groups.stop().await;
                            },
                            ControlFlow::Break(_) => break,
                        }
                    }
                    Some(Err(error)) => {
                        write!(stderr, "Error: {error}\r\n").unwrap();
                        break
                    },
                    None => break,
                }
            }
            _ = timeout => {},
        }

        execute!(stdout, terminal::Clear(terminal::ClearType::CurrentLine)).unwrap();
        write!(stdout, "\r").unwrap();
        stdout.flush().unwrap();

        let mut session_state = rusty_link::SessionState::new();
        link.capture_app_session_state(&mut session_state);

        let time = link.clock_micros();
        let ghost = link.host_to_ghost(time);
        let beat = session_state.beat_at_time(time, 4.0);
        let tempo = session_state.tempo();
        let peers = link.num_peers();
        let is_playing = groups.is_playing();

        write!(
            stdout,
            "id={id}, peers={peers}, is_playing={is_playing}, host={time}, ghost={ghost}, tempo={tempo}, beat={beat}"
        )
        .unwrap();
        stdout.flush().unwrap();
    }

    groups.shutdown().await;
    crossterm::terminal::disable_raw_mode().unwrap();
    println!();
}

#[derive(Debug, Clone)]
enum Action {
    None,
    Start(String),
    Stop,
}

fn on_key_press(ev: Event, id: &mut u64) -> std::ops::ControlFlow<(), Action> {
    let Event::Key(key_event) = ev else {
        return ControlFlow::Continue(Action::None);
    };

    match key_event {
        KeyEvent {
            code: KeyCode::Char('q'),
            ..
        }
        | KeyEvent {
            code: KeyCode::Esc, ..
        }
        | KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => {
            return ControlFlow::Break(());
        }
        KeyEvent {
            code: KeyCode::Up, ..
        } => {
            *id = (*id + 1u64).clamp(0, 9);
        }
        KeyEvent {
            code: KeyCode::Down,
            ..
        } => {
            *id = id.saturating_sub(1);
        }
        KeyEvent {
            code: KeyCode::Char(' '),
            ..
        } => {
            return ControlFlow::Continue(Action::Start(format!("group#{id}")));
        }
        KeyEvent {
            code: KeyCode::Char('s'),
            ..
        } => {
            return ControlFlow::Continue(Action::Stop);
        }
        _ => {}
    }

    ControlFlow::Continue(Action::None)
}

#[derive(Default)]
struct RawTerminalFormatter(
    tracing_subscriber::fmt::format::Format<tracing_subscriber::fmt::format::Full>,
);

impl<S, N> tracing_subscriber::fmt::FormatEvent<S, N> for RawTerminalFormatter
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> tracing_subscriber::fmt::FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        writer: tracing_subscriber::fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        print!("\r\n");
        self.0.format_event(ctx, writer, event)?;
        print!("\r");
        Ok(())
    }
}
