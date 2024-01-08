use std::{
    sync::{mpsc, Arc, Condvar, Mutex, Weak},
    thread::JoinHandle,
    time::Duration,
};

use midir::{MidiOutput, MidiOutputConnection};
use rusty_link::SessionState;
use tracing::{info, warn};

use crate::AppState;

pub struct AudioEngine {
    pub state: Weak<AppState>,
    // TODO: Sefely join and destroy this thread using some sort of contition variable
    // synchronization to break it's work and then join to wait until work is finished.
    #[allow(dead_code)]
    worker: JoinHandle<()>,
    work_in: mpsc::SyncSender<RequestPlay>,
}

struct RequestPlay {
    output: MidiOutputConnection,
    uuid: String,
    app_state: Arc<AppState>,
}

// More info on timing:
// https://majicdesigns.github.io/MD_MIDIFile/page_timing.html
fn audio_engine_main(
    request_play: RequestPlay,
    interrupts: &(Mutex<bool>, Condvar),
) -> Result<(), String> {
    let RequestPlay {
        mut output,
        uuid,
        app_state,
    } = request_play;

    // TODO: Find better solution then checking two times if we have this source
    let midi_sources = app_state.sources.read().unwrap();
    let Some(midi_source) = midi_sources.get(&uuid) else {
        return Err(format!("{uuid} not found. Try reloading page"));
    };

    let midi = midi_source
        .midi()
        .map_err(|err| format!("failed to parse midi: {err}"))?;

    let ticks_per_quater_note = match midi.header.timing {
        midly::Timing::Metrical(tpqn) => tpqn.as_int(),
        _ => return Err("Timecode timing format is not supported".to_string()),
    };

    let mut session_state = SessionState::new();
    let quantum = 4.0;

    *app_state.currently_playing_uuid.write().unwrap() = Some(uuid.to_string());
    *app_state.current_playing_progress.write().unwrap() = (0_usize, midi.tracks[0].len());
    info!("commiting start state");

    app_state.link.capture_app_session_state(&mut session_state);
    session_state.set_is_playing_and_request_beat_at_time(
        true,
        app_state.link.clock_micros() as u64,
        0.0,
        quantum,
    );
    app_state.link.commit_app_session_state(&session_state);

    let mut time_passed = 0.0;
    let mut track = midi.tracks[0].iter().enumerate();

    'audio_loop: loop {
        let Some((nth, (bytes, event))) = track.next() else {
            break;
        };

        *app_state.current_playing_progress.write().unwrap() = (nth, midi.tracks[0].len());

        let (interrupt, interruptable_sleep) = interrupts;
        let interrupted = interrupt.try_lock().map(|x| *x).unwrap_or(false);
        if interrupted {
            break;
        }

        let time_to_wait = event.delta.as_int() as f64 / ticks_per_quater_note as f64;
        time_passed += time_to_wait;

        // TODO: Rust makes a note that condvar shouldn't be use in time critical applications?
        loop {
            app_state.link.capture_app_session_state(&mut session_state);
            let current_time = session_state.beat_at_time(app_state.link.clock_micros(), quantum);

            info!("[passed={time_passed}, current={current_time}] {event:?}");
            if current_time >= time_passed {
                break;
            }

            let sleep_time = (time_passed - current_time) / 120.0 * 60.0;
            let guard = interrupt.lock().unwrap();
            let (interrupted, sleep_result) = interruptable_sleep
                .wait_timeout(guard, Duration::from_secs_f64(sleep_time))
                .unwrap();
            if *interrupted {
                break 'audio_loop;
            }
            if sleep_result.timed_out() {
                break;
            }
        }

        match event.kind {
            midly::TrackEventKind::Meta(meta) => match meta {
                // http://midi.teragonaudio.com/tech/midifile/ppqn.htm
                midly::MetaMessage::Tempo(tempo) => {
                    let tempo: f32 = 60_000_000.0 / (tempo.as_int() as f32);
                    info!("changing tempo to {}", tempo)
                }

                // http://midi.teragonaudio.com/tech/midifile/time.htm
                midly::MetaMessage::TimeSignature(num, den, _, _) => {
                    info!(
                        "time signature is: {num}/{den}",
                        den = 2_usize.pow(den.into())
                    )
                }

                // These are obligatory at the end of track so we don't need to handle them
                midly::MetaMessage::EndOfTrack => {}
                msg => {
                    warn!("unknown meta message: {msg:?}")
                }
            },
            midly::TrackEventKind::Midi {
                channel: _,
                message,
            } => match message {
                // TODO: Remember currenlty played notes so we can unwind this musical stack when
                // we turn it off.
                midly::MidiMessage::NoteOn { .. } | midly::MidiMessage::NoteOff { .. } => {
                    output.send(bytes).unwrap();
                }
                msg => {
                    warn!("unknown midi message: {msg:?}")
                }
            },
            midly::TrackEventKind::SysEx(_) => {
                // TODO: They should probably be forwarded
                warn!("sysex messages are not handled yet");
            }
            midly::TrackEventKind::Escape(_) => {
                // TODO: They should probably be forwarded
                warn!("escape messages are not handled yet");
            }
        }
    }

    // TODO: Cleanup what was playing

    *app_state.currently_playing_uuid.write().unwrap() = None;

    Ok(())
}

impl Default for AudioEngine {
    fn default() -> Self {
        // TODO: We assume that we handle this request relativly quickly (and thus only 1 in queue)
        let (work_in, work) = mpsc::sync_channel(1);

        let mut interrupt: Option<Arc<(Mutex<bool>, Condvar)>> = None;
        let mut current_worker: Option<JoinHandle<()>> = None;

        let worker = std::thread::spawn(move || {
            while let Ok(request) = work.recv() {
                if let Some(interrupt) = interrupt {
                    *interrupt.0.lock().unwrap() = true;
                    interrupt.1.notify_one();
                    if let Some(current_worker) = current_worker {
                        current_worker.join().unwrap();
                    }
                }

                interrupt = Some(Arc::new((Mutex::new(false), Condvar::new())));
                let worker_interrupt = interrupt.clone().unwrap();
                let worker = std::thread::spawn(move || {
                    if let Err(err) = audio_engine_main(request, &worker_interrupt) {
                        crate::error!("{err:#}")
                    }
                });
                current_worker = Some(worker);
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
    info!(
        "outputing to output port: {}",
        midi_out.port_name(midi_port).unwrap()
    );
    let conn_out = midi_out
        .connect(midi_port, /* TODO: Better name */ "play")
        .map_err(|err| format!("failed to connect to midi port: {err}"))?;

    // TODO: This is wrong approach, we should select what will be played, not what to play now.
    app_state
        .audio_engine
        .write()
        .unwrap()
        .work_in
        .send(RequestPlay {
            output: conn_out,
            uuid: uuid.to_string(),
            app_state: app_state.clone(),
        })
        .map_err(|err| format!("failed to send job: {err}"))?;

    Ok(())
}
