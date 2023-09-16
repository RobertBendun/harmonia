use std::{net::SocketAddr, path::PathBuf, sync::{Arc, RwLock}};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        ConnectInfo, Multipart, Path, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::{get, post},
    Json, Router, TypedHeader, Extension,
};
use midly::Smf;
use tower_http::{
    services::ServeDir,
    trace::{DefaultMakeSpan, TraceLayer},
};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

type MidiSources = std::collections::HashMap<String, ()>;

#[tokio::main]
async fn main() {
    let midi_sources = Arc::new(RwLock::new(MidiSources::new()));

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "example_websockets=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let public_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("public");

    let app = Router::new()
        .fallback_service(ServeDir::new(public_dir).append_index_html_on_directories(true))
        .route("/ws", get(websocket_handler))
        .route("/add-midi-sources", post(add_midi_sources_handler))
        .route("/play/:uuid", post(play_handler))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .layer(Extension(midi_sources));

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    info!("listening on http://{addr}");
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

async fn play_handler(
    Path(uuid): Path<String>,
    midi_sources: Extension<Arc<RwLock<MidiSources>>>,
) {
    if midi_sources.read().unwrap().contains_key(&uuid) {
        println!("playing {uuid}");
    } else {
        println!("not found");
    }
}

#[derive(serde::Serialize)]
struct MidiSourceLoaded {
    file_name: String,
    format: String,
    tracks_count: usize,
    uuid: String,
}

async fn add_midi_sources_handler(
    midi_sources: Extension<Arc<RwLock<MidiSources>>>,
    mut multipart: Multipart,
) -> Json<Vec<Result<MidiSourceLoaded, String>>> {
    let mut statuses = vec![];


    while let Some(field) = multipart.next_field().await.unwrap() {
        // TODO: Check that this is the name that we are expecting
        let _name = field.name().unwrap().to_string();
        // TODO: Better default file name
        let file_name = field.file_name().unwrap_or("<unknown>").to_string();
        let data = field.bytes().await.unwrap().to_vec();

        statuses.push(match Smf::parse(&data) {
            Ok(midi) => {
                let uuid = uuid::Uuid::new_v4().to_string();
                let mut midi_sources = midi_sources.write().unwrap();
                midi_sources.insert(uuid.clone(), ());

                Ok(MidiSourceLoaded {
                    file_name,
                    format: match midi.header.format {
                        midly::Format::SingleTrack | midly::Format::Sequential => "sequential",
                        midly::Format::Parallel => "parallel",
                    }
                    .to_string(),
                    tracks_count: midi.tracks.len(),
                    uuid,
                })
            }
            Err(err) => Err(err.to_string()),
        });
    }
    Json(statuses)
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
