use std::{
    sync::{mpsc, Arc, Weak},
    thread::JoinHandle,
    time::Duration,
};

use midir::{MidiOutput, MidiOutputConnection};

use crate::AppState;

pub struct AudioEngine {
    pub state: Weak<AppState>,
    // TODO: Sefely join and destroy this thread using some sort of contition variable
    // synchronization to break it's work and then join to wait until work is finished.
    #[allow(dead_code)]
    worker: JoinHandle<()>,
    work_in: mpsc::SyncSender<Job>,
}

struct Job {
    output: MidiOutputConnection,
    uuid: String,
    app_state: Arc<AppState>,
}

fn audio_engine_main(job: Job) -> Result<(), String> {
    let Job {
        mut output,
        uuid,
        app_state,
    } = job;

    // TODO: Find better solution then checking two times if we have this source
    let midi_sources = app_state.sources.read().unwrap();
    let Some(midi_source) = midi_sources.get(&uuid) else {
        return Err(format!("{uuid} not found. Try reloading page"));
    };

    let midi = midi_source
        .midi()
        .map_err(|err| format!("failed to parse midi: {err}"))?;

    for (bytes, event) in midi.tracks[0].iter() {
        match event.kind {
            midly::TrackEventKind::Midi {
                channel: _,
                message,
            } => match message {
                midly::MidiMessage::NoteOn { .. } | midly::MidiMessage::NoteOff { .. } => {
                    output.send(bytes).unwrap();
                    std::thread::sleep(Duration::from_secs(1));
                }
                _ => {}
            },
            _ => {}
        }
    }

    Ok(())
}

impl Default for AudioEngine {
    fn default() -> Self {
        // TODO: We assume that we handle this request relativly quickly (and thus only 1 in queue)
        let (work_in, work) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            while let Ok(job) = work.recv() {
                if let Err(err) = audio_engine_main(job) {
                    crate::error!(err);
                }
            }
        });

        Self {
            state: Default::default(),
            worker,
            work_in,
        }
    }
}

pub async fn play(app_state: Arc<AppState>, uuid: &str) -> Result<(), String> {
    let midi_sources = app_state.sources.read().unwrap();
    let Some(midi_source) = midi_sources.get(uuid) else {
        return Err(format!("{uuid} not found. Try reloading page"));
    };

    midi_source
        .midi()
        .map_err(|err| format!("failed to parse midi: {err}"))?;

    let midi_out = MidiOutput::new("harmonia")
        .map_err(|err| format!("failed to create midi output port: {err}"))?;
    // TODO: Use associated port
    let midi_port = &midi_out.ports()[0];
    crate::info!(
        "outputing to output port: {}",
        midi_out.port_name(midi_port).unwrap()
    );
    let conn_out = midi_out
        .connect(midi_port, /* TODO: Better name */ "play")
        .map_err(|err| format!("failed to connect to midi port: {err}"))?;

    app_state
        .audio_engine
        .write()
        .unwrap()
        .work_in
        .send(Job {
            output: conn_out,
            uuid: uuid.to_string(),
            app_state: app_state.clone(),
        })
        .map_err(|err| format!("failed to send job: {err}"))?;

    Ok(())
}
