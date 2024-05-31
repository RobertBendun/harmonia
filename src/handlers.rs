// TODO:  This triplets of {SetX, midi_set_x_for_source, render_x_cell} maybe should be
// consolidated

use crate::{audio_engine, block, cache_path, AppState, Version};
use axum::{
    body::{Bytes, Full},
    extract::{Multipart, Path, State},
    http::{
        header::{CONTENT_DISPOSITION, CONTENT_TYPE},
        Response, StatusCode,
    },
    Form,
};
use maud::{html, Markup, PreEscaped, DOCTYPE};
use midir::MidiOutput;
use rusty_link::SessionState;
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::{path::PathBuf, sync::Arc};
use tracing::{error, info};

pub async fn index(app_state: State<Arc<AppState>>) -> Markup {
    html! {
        (DOCTYPE);
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "Harmonia" }
                meta name="viewport" content="width=device-width, initial-scale=1";
                script src="index.js" {}
                script src="htmx.min.js" {}
                link rel="stylesheet" href="index.css";
            }
            body {
                noscript {
                    div style="font-weight: bold; color: red; font-size: 1.1em; max-width: 800px; margin: 1em auto 1em auto" {
                        "This is a web application, and thus requires JavaScript to work. But fear not!
                         This code is a free software (AGPL 3+) with at least open source dependencies:";
                        ul {
                            li { a href="https://github.com/RobertBendun/harmonia" { "harmonia" }  " - app that you are seeing now"; }
                            li { a href="https://htmx.org/" { "htmx" } }
                        }
                    }
                }

                header {
                    h1 { "Harmonia" }
                    div {
                        div {
                            "Version: ";
                            (Version::default());
                        }
                        div {
                            "Data: ";
                            ({
                                let cache = cache_path();
                                let c: &std::path::Path = &cache;

                                // TODO: On Windows don't use "~" as HOME since it may not work,
                                // use environment variable instead
                                dirs::home_dir()
                                    .and_then(|home| c.strip_prefix(home).ok())
                                    .map(|p| PathBuf::from("~").join(p))
                                    .unwrap_or(cache)
                                    .to_str()
                                    .unwrap()
                                    .to_owned()
                            });
                        }
                    }
                }

                aside {
                    table id="status" {
                        (runtime_status(app_state.clone()).await);
                    }
                    div {
                        label for="midi" { "New MIDI" }
                        input
                            style="display: none"
                            type="file"
                            id="midi"
                            name="midi"
                            multiple
                            accept="audio/midi"
                            hx-put="/blocks/midi"
                            hx-target="#blocks"
                            hx-swap="innerHTML"
                            hx-encoding="multipart/form-data";
                        button {
                            // TODO: Handle SHM adding
                            "New SHM"
                        }
                        button onclick="toggle_delete(this)" {
                            "Delete mode"
                        }
                    }
                }

                main id="blocks" {
                    (blocks(app_state.clone()).await)
                }

                details class="midi-outputs" {
                    summary { "MIDI Outputs" }
                    p {
                        "Refresh page (F5 or CTRL-R) to refresh list of available MIDI ports";
                    }
                    (midi_ports(app_state.clone()).await)
                }

                details class="system-information" {
                    summary { "System information" }
                    (system_information(app_state.clone()).await);
                }

                footer {
                    button
                        hx-post="/interrupt"
                        hx-swap="none"
                        style="grid-area: stop"
                    {
                        (PreEscaped("&#x23f8;"))
                    }
                    (playing_status(app_state.clone()).await)
                }
            }
        }
    }
}

pub async fn runtime_status(app_state: State<Arc<AppState>>) -> Markup {
    let mut session_state = SessionState::default();
    let active = app_state.link.is_enabled();
    app_state.link.capture_app_session_state(&mut session_state);
    let time = app_state.link.clock_micros();

    // TODO: Move quantum to state
    let quantum = 1.0;
    let beat = session_state.beat_at_time(time, quantum);
    let peers = app_state.link.num_peers();

    html! {
        tr {
            th id="app-health" colspan="2" {
                @if active { "Synchronized" }
                @else { "ERROR" }
            }
        }
        tr { th { "Peers" } td { (peers) } }
        tr { th { "Beat" } td { (format!("{beat:.1}")) } }
        tr { th { "BPM" } td { (session_state.tempo()) } }
    }
}

async fn playing_status(app_state: State<Arc<AppState>>) -> Markup {
    let _ = app_state;
    html! {}
}

async fn system_information(app_state: State<Arc<AppState>>) -> Markup {
    let port = app_state.port;
    let mut interfaces = match local_ip_address::list_afinet_netifas() {
        Ok(list) => list,
        Err(err) => {
            error!("failed to retrive local ips: {err}");
            return html! {
                p {
                    "failed to retrive local ips"
                }
            };
        }
    };

    interfaces.sort_by(|(if1, _), (if2, _)| if1.cmp(if2));

    let hostname = gethostname::gethostname();
    let hostname = hostname.to_string_lossy();

    html! {
        p {
            "Hostname: "; a href=(format!("http://{hostname}:{port}")) {
                (hostname)
            }
        }

        @if let Ok(local_ip) = local_ip_address::local_ip() {
            p {
                "Local IP: ";
                (local_ip)
            }
        } @else {
            ul {
                @for (iface, ip) in interfaces {
                    @if !ip.is_loopback() {
                        li {
                            (format!("{iface} -"));
                            a href=(format!("http://{ip}:{port}")) {
                                (ip);
                            }
                        }
                    }
                }
            }
        }

    }
}

pub async fn midi_ports(State(app_state): State<Arc<AppState>>) -> Markup {
    let out = MidiOutput::new("harmonia").unwrap();

    let mut midi_conn = app_state.connection.write().unwrap();
    midi_conn.refresh();

    let ports = midi_conn
        .ports
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

async fn blocks(app_state: State<Arc<AppState>>) -> Markup {
    use crate::block::Content;

    let blocks = app_state.blocks.read().unwrap();
    let mut orderered_blocks: Vec<_> = blocks.iter().collect();

    orderered_blocks.sort_by(|(_, lhs), (_, rhs)| match (lhs.order, rhs.order) {
        (Some(lhs), Some(rhs)) => lhs.cmp(&rhs),
        (Some(_), _) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => match (&lhs.content, &rhs.content) {
            (Content::Midi(lhs), Content::Midi(rhs)) => lhs.file_name.cmp(&rhs.file_name),
            (Content::SharedMemory { path: lhs }, Content::SharedMemory { path: rhs }) => {
                lhs.cmp(rhs)
            }
            (Content::SharedMemory { .. }, Content::Midi(_)) => std::cmp::Ordering::Less,
            (Content::Midi(_), Content::SharedMemory { .. }) => std::cmp::Ordering::Greater,
        },
    });

    html! {
        @for (uuid, block) in orderered_blocks.iter() {
            section class="block" {
                button
                    class="delete-mode icon-control"
                    hx-delete=(format!("/blocks/{uuid}"))
                    hx-swap="innerHTML"
                    hx-target="#blocks"
                {
                    "🗑️"
                }
                button
                    hx-post=(format!("/blocks/play/{uuid}"))
                    hx-swap="none"
                    class="icon-control"
                {
                    "▶"
                }
                div {
                    @match &block.content {
                        Content::Midi(source) => {
                            a href=(format!("/blocks/{uuid}")) { (source.file_name) }
                        }
                        Content::SharedMemory { path } => (path),
                    }
                }

                @if let Content::Midi(source) = &block.content {
                    (port_cell(uuid, source.associated_port))
                }

                (group(uuid, &block.group));
                (keybind(uuid, &block.keybind));
            }
        }
    }
}

fn group(uuid: &str, group: &str) -> Markup {
    html! {
        input
            type="text" value=(group)
            pattern=(format!("(\\w| ){{0,{}}}", linky_groups::MAX_GROUP_ID_LENGTH))
            maxlength=(linky_groups::MAX_GROUP_ID_LENGTH)
            name="group"
            placeholder="Group"
            hx-target="this"
            hx-post=(format!("/blocks/set-group/{uuid}"));
    }
}

#[derive(Deserialize)]
pub struct SetGroup {
    pub group: String,
}

pub async fn set_group(
    app_state: State<Arc<AppState>>,
    Path(uuid): Path<String>,
    Form(SetGroup {
        group: group_to_set,
    }): Form<SetGroup>,
) -> Result<Markup, StatusCode> {
    let response = {
        let mut blocks = app_state.blocks.write().unwrap();

        let Some(midi_source) = blocks.get_mut(&uuid) else {
            error!("block#{uuid} not found");
            return Err(StatusCode::NOT_FOUND);
        };

        // TODO: Unnesesary string allocation
        midi_source.group = if group_to_set.len() > linky_groups::MAX_GROUP_ID_LENGTH {
            let mut cut = linky_groups::MAX_GROUP_ID_LENGTH;
            while !group_to_set.is_char_boundary(cut) {
                cut -= 1;
            }
            &group_to_set[..cut]
        } else {
            &group_to_set[..]
        }
        .to_owned();

        tracing::info!(
            "Switched block#{uuid} to group {group:?}",
            group = midi_source.group
        );
        Ok(group(&uuid, &midi_source.group))
    };

    if let Err(err) = app_state.remember_current_blocks() {
        error!("block_set_group failed to remember current sources: {err:#}")
    }
    response
}

fn keybind(uuid: &str, keybind: &str) -> Markup {
    html! {
        input
            name="keybind"
            data-uuid=(uuid)
            onchange="update_key_binding(this)"
            type="text"
            hx-post=(format!("/blocks/set-keybind/{uuid}"))
            hx-swap="none"
            placeholder="Keybind"
            value=(keybind);
    }
}

#[derive(Deserialize)]
pub struct SetKeybind {
    pub keybind: String,
}

pub async fn set_keybind(
    app_state: State<Arc<AppState>>,
    Path(uuid): Path<String>,
    Form(SetKeybind { keybind }): Form<SetKeybind>,
) -> StatusCode {
    {
        let mut midi_sources = app_state.blocks.write().unwrap();

        let Some(block) = midi_sources.get_mut(&uuid) else {
            error!("block#{uuid} not found");
            return StatusCode::NOT_FOUND;
        };

        info!("Changing keybind for block#{uuid} to {keybind}");
        block.keybind = keybind;
    }

    if let Err(err) = app_state.remember_current_blocks() {
        error!("set_keybind failed to remember current sources: {err:#}")
    }

    StatusCode::OK
}

fn port_cell(uuid: &str, associated_port: usize) -> Markup {
    html! {
        input
            type="number" value=(format!("{}", associated_port + 1))
            name="port"
            hx-target="this"
            hx-post=(format!("/blocks/set-port/{uuid}"));
    }
}

#[derive(Deserialize)]
pub struct SetPort {
    pub port: usize,
}

pub async fn set_port_for_midi(
    app_state: State<Arc<AppState>>,
    Path(uuid): Path<String>,
    Form(SetPort { port }): Form<SetPort>,
) -> Result<Markup, StatusCode> {
    let mut blocks = app_state.blocks.write().unwrap();

    let Some(block) = blocks.get_mut(&uuid) else {
        error!("block#{uuid} was not found");
        return Err(StatusCode::NOT_FOUND);
    };

    let block::Content::Midi(ref mut midi) = block.content else {
        error!("block#{uuid} is not a MIDI source");
        return Err(StatusCode::BAD_REQUEST);
    };

    let min = 1_usize;
    let max = app_state.connection.read().unwrap().ports.len();
    if port < min || port > max {
        error!("port number should be between {min} and {max}");
        return Ok(port_cell(&uuid, midi.associated_port));
    }

    info!("setting port {port} for {uuid}");
    midi.associated_port = port - 1;
    Ok(port_cell(&uuid, midi.associated_port))
}

pub async fn download_block_content(
    app_state: State<Arc<AppState>>,
    Path(uuid): Path<String>,
) -> Response<Full<Bytes>> {
    let not_found = || {
        let mut response = Response::new(Full::from("not found"));
        *response.status_mut() = StatusCode::NOT_FOUND;
        response
            .headers_mut()
            .insert(CONTENT_TYPE, "text/html".parse().unwrap());
        response
    };

    let blocks = app_state.blocks.read().unwrap();
    let Some(block) = blocks.get(&uuid) else {
        error!("block#{uuid} not found");
        return not_found();
    };

    match &block.content {
        block::Content::SharedMemory { .. } => not_found(),
        block::Content::Midi(midi_source) => {
            // TODO: Unnesesary clone?
            let mut response = Response::new(Full::from(midi_source.bytes.clone()));
            let headers = &mut response.headers_mut();
            headers.insert(
                CONTENT_DISPOSITION,
                format!("attachement; filename=\"{}\"", midi_source.file_name)
                    .parse()
                    .unwrap(),
            );
            headers.insert(CONTENT_TYPE, "audio/midi".parse().unwrap());
            response
        }
    }
}

pub async fn remove_block(app_state: State<Arc<AppState>>, Path(uuid): Path<String>) -> Markup {
    {
        let mut sources = app_state.blocks.write().unwrap();
        sources.remove(&uuid);
    }
    if let Err(err) = app_state.remember_current_blocks() {
        error!("remove_midi_source_handler failed to remember current sources: {err:#}")
    }

    blocks(app_state).await
}

pub async fn play(State(app_state): State<Arc<AppState>>, Path(uuid): Path<String>) {
    let _ = audio_engine::play(app_state.clone(), &uuid).await;
}

pub async fn interrupt(State(app_state): State<Arc<AppState>>) {
    if let Err(error) = audio_engine::interrupt(app_state).await {
        tracing::error!("failed to interrupt: {error}");
    }
}

#[derive(Deserialize)]
pub struct AddSharedMemoryBlock {
    path: String,
}

pub async fn add_new_shered_memory_block(
    State(app_state): State<Arc<AppState>>,
    Form(AddSharedMemoryBlock { path }): Form<AddSharedMemoryBlock>,
) -> Markup {
    let mut hasher = Sha1::new();
    hasher.update(path.as_bytes());
    let uuid = hex::encode(hasher.finalize());

    let content = block::Content::SharedMemory { path };

    let block = block::Block {
        content,
        group: Default::default(),
        keybind: Default::default(),
        order: Default::default(),
    };

    {
        let blocks = &mut app_state.blocks.write().unwrap();
        blocks.insert(format!("shm-{uuid}"), block);
    }

    if let Err(err) = app_state.remember_current_blocks() {
        error!("add_new_shered_memory_block failed to remember current sources: {err:#}")
    }

    blocks(axum::extract::State(app_state)).await
}

pub async fn add_new_midi_source_block(
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
        let uuid = hex::encode(hasher.finalize());

        let midi_source = block::MidiSource {
            bytes: data,
            file_name: file_name.clone(),
            associated_port: 0,
        };

        let block = block::Block {
            content: block::Content::Midi(midi_source),
            group: Default::default(),
            keybind: Default::default(),
            order: Default::default(),
        };

        let midi_sources = &mut app_state.blocks.write().unwrap();
        midi_sources.insert(format!("midi-{uuid}"), block);
    }

    if let Err(err) = app_state.remember_current_blocks() {
        error!("add_new_midi_source_block failed to remember current sources: {err:#}")
    }

    blocks(axum::extract::State(app_state)).await
}
