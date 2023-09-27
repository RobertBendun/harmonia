use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        ConnectInfo, Multipart, Path, WebSocketUpgrade, State,
    },
    response::IntoResponse,
    routing::{get, post},
    Json, Router, TypedHeader,
};
use midir::{MidiOutput, MidiOutputPort};
use midly::SmfBytemap;
use tower_http::{
    services::ServeDir,
    trace::{DefaultMakeSpan, TraceLayer},
};
use tracing::{error, info, instrument};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use maud::{html, Markup, DOCTYPE};

use sha1::{Sha1, Digest};

use base64ct::{Base64, Encoding};

struct MidiSource {
    bytes: Vec<u8>,
    #[allow(dead_code)]
    file_name: String,
}

impl MidiSource {
    fn midi(&self) -> Result<SmfBytemap<'_>, midly::Error> {
        SmfBytemap::parse(&self.bytes)
    }

    #[instrument]
    fn play() {}
}

type MidiSources = HashMap<String, MidiSource>;

struct MidiConnection {
    ports: Vec<MidiOutputPort>,
}

impl Default for MidiConnection {
    fn default() -> Self {
        // TODO: Is it valid to create a new MidiOutput per use? Maybe we should create only one
        // MidiOutput port per application.
        let out = MidiOutput::new("harmonia").unwrap();

        Self {
            ports: out.ports(),
        }
    }
}

impl MidiConnection {
    fn refresh(&mut self) {
        // TODO: Is it valid to create a new MidiOutput per use? Maybe we should create only one
        // MidiOutput port per application.
        let out = MidiOutput::new("harmonia").unwrap();
        self.ports = out.ports();
    }
}

#[derive(Default)]
struct AppState {
    sources: RwLock<MidiSources>,
    connection: RwLock<MidiConnection>,
}

#[tokio::main]
async fn main() {
    let do_help = std::env::args().any(|param| &param == "--help" || &param == "-h");
    let do_open = std::env::args().any(|param| &param == "--open");

    if do_help {
        help_and_exit();
    }

    let app_state = Arc::new(AppState::default());

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "harmonia=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("starting up commit {}", env!("GIT_INFO"));

    let public_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("public");

    let app = Router::new()
        .fallback_service(ServeDir::new(public_dir))
        .route("/api/health", get(health_handler))
        .route("/api/ws", get(websocket_handler))
        .route("/midi/add", post(midi_add_handler))
        .route("/api/midi/play/:uuid", post(midi_play_handler))
        .route("/midi/ports", get(midi_ports_handler))
        .route("/version", get(version_handler))
        .route("/", get(index_handler))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .with_state(app_state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    info!("listening on http://{addr}");
    let server =
        axum::Server::bind(&addr).serve(app.into_make_service_with_connect_info::<SocketAddr>());

    if do_open {
        info!("opening UI in default browser");
        open::that_detached(format!("http://{addr}")).unwrap();
    }

    server.await.unwrap();
}

fn help_and_exit() -> ! {
    println!("harmonia [--open] [--help]");
    println!("  --open - opens UI in default browser");
    println!("  --help - prints this message");
    std::process::exit(0);
}

async fn midi_sources_render(
    app_state: State<Arc<AppState>>,
) -> Markup {
    let midi_sources = app_state.sources.read().unwrap();

    html! {
        @for (uuid, source) in midi_sources.iter() {
            div data-uuid=(uuid) {
                h3 { (source.file_name) }
                @match source.midi() {
                    Ok(midi) => p {
                        "Format: ";
                        ({
                            match midi.header.format {
                                midly::Format::Sequential | midly::Format::SingleTrack => "sequential",
                                midly::Format::Parallel => "parallel",
                            }
                        });
                        ", tracks count: ";
                        (midi.tracks.len());
                    },
                    Err(err) => p {
                        "Failed to parse MIDI file: ";
                        (err);
                    },
                }
            }
        }
    }
}

async fn index_handler(
    app_state: State<Arc<AppState>>,
) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            (DOCTYPE)
            head {
                meta charset="utf-8";
                title { "Harmonia control panel" }
                script src="index.js" {}
                script src="htmx.min.js" {}
            }
            body {
                header {
                    h1 { "Harmonia control panel" }
                    "Status: "
                    span id="app-health" {}
                    br;
                    (version_handler().await);
                }
                main {
                    h2 { "MIDI ports" }
                    button hx-get="/midi/ports" hx-target="#midi-ports" hx-swap="innerHTML" {
                        "Refresh"
                    }
                    div id="midi-ports" {
                        (midi_ports_handler(app_state.clone()).await);
                    }
                    h2 { "MIDI sources" }
                    form
                        hx-post="/midi/add"
                        hx-target="#midi-sources-list"
                        hx-swap="innerHTML"
                        hx-encoding="multipart/form-data"
                    {
                        input id="midi-sources-input" name="files" type="file" multiple accept="audio/midi";
                        button { "Upload" }
                    }
                    div id="midi-sources-list" {
                        (midi_sources_render(app_state).await);
                    }
                }
            }
        }
    }
}

async fn health_handler() -> &'static str {
    "Hi"
}

async fn version_handler() -> Markup {
    html! {
        (format!("Version: {}+{}", env!("CARGO_PKG_VERSION"), env!("GIT_INFO")));
    }
}

async fn midi_ports_handler(
    State(app_state): State<Arc<AppState>>,
) -> Markup {
    let out = MidiOutput::new("harmonia").unwrap();

    let mut midi_conn = app_state.connection.write().unwrap();
    midi_conn.refresh();

    let ports = midi_conn.ports
            .iter()
            .filter_map(|port| Result::ok(out.port_name(port)));

    html! {
        ol {
            @for port_name in ports {
                li { (port_name) }
            }
        }
    }
}

// use axum::debug_handler;
// #[debug_handler]
async fn midi_play_handler(
    State(app_state): State<Arc<AppState>>,
    Path(uuid): Path<String>,
) -> Json<()> {
    let midi_sources = app_state.sources.read().unwrap();
    let Some(midi_source) = midi_sources.get(&uuid) else {
        println!("not found");
        return Json(());
    };
    let midi = midi_source.midi().unwrap();

    let midi_out = MidiOutput::new("harmonia").unwrap();
    let midi_port = &midi_out.ports()[0];
    info!(
        "connected to output port: {}",
        midi_out.port_name(midi_port).unwrap()
    );
    let mut conn_out = midi_out
        .connect(midi_port, /* TODO: Better name */ "play")
        .unwrap();

    for (bytes, event) in midi.tracks[0].iter() {
        match event.kind {
            midly::TrackEventKind::Midi {
                channel: _,
                message,
            } => match message {
                midly::MidiMessage::NoteOn { .. } | midly::MidiMessage::NoteOff { .. } => {
                    conn_out.send(bytes).unwrap();
                    std::thread::sleep(Duration::from_secs(1));
                }
                _ => {}
            },
            _ => {}
        }
    }

    Json(())
}

async fn midi_add_handler(
    State(app_state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Markup {
    while let Some(field) = multipart.next_field().await.unwrap() {
        // TODO: Check that this is the name that we are expecting
        let _name = field.name().unwrap().to_string();
        // TODO: Better default file name
        let file_name = field.file_name().unwrap_or("<unknown>").to_string();
        let data = field.bytes().await.unwrap().to_vec();
        let mut hasher = Sha1::new();
        hasher.update(&data);
        let uuid = Base64::encode_string(&hasher.finalize());

        let midi_source = MidiSource {
            bytes: data,
            file_name: file_name.clone(),
        };

        let midi_sources = &mut app_state.sources.write().unwrap();
        midi_sources.insert(uuid, midi_source);
    }

    midi_sources_render(axum::extract::State(app_state)).await
}

// For expanding this websocket buisness see: https://github.com/tokio-rs/axum/blob/main/examples/websockets/src/main.rs
async fn websocket_handler(
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        "unknown user agent".to_string()
    };
    info!("websocket connect: addr={addr}, user_agent={user_agent}");
    ws.on_upgrade(move |socket| handle_socket(socket, addr))
}

async fn handle_socket(mut socket: WebSocket, addr: SocketAddr) {
    if socket
        .send(Message::Text("hello, world".to_string()))
        .await
        .is_ok()
    {
        info!("websocket send to {addr} message: ");
    } else {
        return;
    }

    while let Some(msg) = socket.recv().await {
        let msg = match msg {
            Ok(msg) => msg,
            Err(err) => {
                error!("failed to receive: {err}");
                continue;
            }
        };
        match msg {
            Message::Text(msg) => println!("Received message: {msg}"),
            Message::Binary(bin) => println!("Binary message of length: {}", bin.len()),
            Message::Close(_) => return,
            Message::Ping(_) | Message::Pong(_) => {}
        }
    }
}
