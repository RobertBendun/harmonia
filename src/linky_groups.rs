use rusty_link::{AblLink, SessionState};
use serde::{Deserialize, Serialize};
use std::{sync::atomic, sync::Arc};

mod net;

type GroupId = [u8; 15];

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
struct GroupFrame {
    /// Magic sequence distinguishing packets
    magic: [u8; 4],

    version: u8,

    /// Group identificator used to distinguish between concurrently going groups
    group_id: GroupId,

    /// Timestamp in microseconds that is a reference point using global host time for when group
    /// was started
    timestamp: i64,
}

impl std::fmt::Display for GroupFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Group(version = {version}, id = ",
            version = self.version
        )?;
        let group_id = &self.group_id[..self
            .group_id
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(self.group_id.len())];
        if let Ok(group_id) = std::str::from_utf8(group_id) {
            write!(f, "{group_id:?}")?;
        } else {
            write!(f, "{group_id:?}", group_id = self.group_id)?;
        }
        write!(f, ", timestamp = {timestamp})", timestamp = self.timestamp)
    }
}

impl GroupFrame {
    fn new(group_id: GroupId, timestamp: i64) -> Self {
        Self {
            magic: *b"grup",
            version: 1,
            group_id,
            timestamp,
        }
    }

    fn is_supported(&self) -> bool {
        self.magic == *b"grup" && self.version == 1
    }
}

pub struct Groups {
    /// Listening task that receives group messages
    listener: tokio::task::JoinHandle<()>,

    /// State consolidation worker
    worker: tokio::task::JoinHandle<()>,

    /// Channel used to issue cancelation request
    cancel: tokio::sync::mpsc::Sender<()>,

    /// Channel for state updates
    actions: tokio::sync::mpsc::Sender<Action>,

    /// Used Ableton Link instance for time synchronization
    link: std::sync::Arc<rusty_link::AblLink>,

    /// Is set when there is a group in which we are playing.
    is_playing: Arc<atomic::AtomicBool>,
}

#[derive(Debug)]
pub enum Error {
    GroupIdTooLong,
}

impl Groups {
    pub async fn start(&self, group_id_str: &str) -> Result<(), Error> {
        let mut group_id: GroupId = Default::default();
        if group_id_str.len() > group_id.len() {
            return Err(Error::GroupIdTooLong);
        }
        group_id[..group_id_str.len()].copy_from_slice(group_id_str.as_bytes());

        let host_time = self.link.clock_micros();
        let ghost_time = self.link.host_to_ghost(host_time);
        let frame = GroupFrame::new(group_id, ghost_time);
        self.actions
            .send(Action::Start(frame))
            .await
            .expect("receiver will never be closed unless in destructor");
        Ok(())
    }

    pub async fn stop(&self) {
        self.actions
            .send(Action::Stop)
            .await
            .expect("receiver will never be closed unless in destructor");
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(atomic::Ordering::SeqCst)
    }

    pub async fn shutdown(self) {
        tracing::debug!("Issuing shutdown");
        self.cancel.send(()).await.unwrap();
        self.actions.send(Action::Quit).await.unwrap();

        self.listener.await.unwrap();
        self.worker.await.unwrap();
    }
}

enum Action {
    /// Start playing in the provided group
    Start(GroupFrame),

    /// Join the provided group if it matches currently played
    Join(GroupFrame),

    /// Stop playing in the provided group (leave group)
    Stop,

    /// Quit listening
    Quit,
}

async fn negotatior(
    mut state: tokio::sync::mpsc::Receiver<Action>,
    link: Arc<AblLink>,
    connection: Arc<net::Sockets>,
    is_playing: Arc<std::sync::atomic::AtomicBool>,
) {
    use tokio::time::{Duration, Instant};

    let mut current_group = None;
    let mut last_send_time = Instant::now();

    const TIMEOUT_DURATION: Duration = Duration::from_millis(50);
    const QUANTUM: f64 = 1.0;

    loop {
        let request = if current_group.is_some() {
            let timeout = tokio::time::sleep(TIMEOUT_DURATION);
            tokio::select! {
                request = state.recv() => request,
                _ = timeout => None,
            }
        } else {
            state.recv().await
        };

        if let Some(request) = request {
            match request {
                Action::Start(frame) => {
                    current_group = Some(frame);

                    let host_time = link.ghost_to_host(frame.timestamp);

                    tracing::info!("starting {frame}");
                    let mut session_state = SessionState::new();
                    link.capture_app_session_state(&mut session_state);
                    session_state.request_beat_at_time(0.0, host_time, QUANTUM);
                    link.commit_app_session_state(&session_state);

                    is_playing.store(true, atomic::Ordering::SeqCst);
                }
                Action::Join(frame) => {
                    if let Some(current_frame) = current_group {
                        // TODO: Add tolerance interval like Ableton/Link
                        if current_frame.group_id == frame.group_id
                            && current_frame.timestamp > frame.timestamp
                        {
                            let foreign_host_time = link.ghost_to_host(frame.timestamp);
                            let my_host_time = link.clock_micros();

                            let mut session_state = SessionState::new();
                            link.capture_app_session_state(&mut session_state);

                            let beat_difference =
                                session_state.beat_at_time(foreign_host_time, QUANTUM);
                            let current_beat = session_state.beat_at_time(my_host_time, QUANTUM);
                            let desired_beat = current_beat - beat_difference;

                            tracing::info!("Transitioning from {current_beat} to {desired_beat} with frame {frame}");

                            session_state.request_beat_at_time(desired_beat, my_host_time, QUANTUM);
                            link.commit_app_session_state(&session_state);
                            current_group = Some(frame);
                        }
                    }
                }
                Action::Stop => {
                    current_group.take();
                    is_playing.store(false, atomic::Ordering::SeqCst);
                    tracing::info!("Stopping playing current group");
                }
                Action::Quit => break,
            }
        }

        if last_send_time.elapsed() >= TIMEOUT_DURATION {
            if let Some(frame) = current_group {
                connection.send(frame).await;
                last_send_time = tokio::time::Instant::now();
            }
        }
    }
}

pub fn listen(link: std::sync::Arc<rusty_link::AblLink>) -> Groups {
    let connection = Arc::new(net::Sockets::bind());
    let (cancel, wait_for_cancel) = tokio::sync::mpsc::channel(1);
    let (send_action, state) = tokio::sync::mpsc::channel(4);
    let is_playing = Arc::new(atomic::AtomicBool::new(false));

    let worker_connection = connection.clone();
    let listener_connection = connection;

    Groups {
        actions: send_action.clone(),
        link: link.clone(),
        is_playing: is_playing.clone(),
        listener: tokio::spawn(async move {
            listener_connection
                .listen(send_action.clone(), wait_for_cancel)
                .await;
        }),
        worker: tokio::spawn(async move {
            negotatior(state, link, worker_connection, is_playing).await;
        }),
        cancel,
    }
}
