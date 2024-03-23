use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{atomic::AtomicBool, Arc, Barrier, RwLock},
};

use tracing::info;

pub struct StartListSession {
    #[allow(dead_code)]
    state: StatePtr,
    quit: Arc<AtomicBool>,
    pub listener: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for StartListSession {
    fn drop(&mut self) {
        self.quit.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(listener) = self.listener.take() {
            tokio::spawn(async move { listener.await.unwrap() });
        }
    }
}

impl StartListSession {
    pub fn listen() -> Self {
        let state = Arc::new(RwLock::new(Default::default()));
        let init_end = Arc::new(Barrier::new(2));
        let quit = Arc::new(AtomicBool::new(false));
        let mine_init_end = init_end.clone();

        let session = Self {
            state: state.clone(),
            quit: quit.clone(),
            listener: Some(tokio::spawn(
                async move { listen(state, init_end, quit).await },
            )),
        };

        // Wait so listener can setup all that it needs
        mine_init_end.wait();
        session
    }
}

struct State {
    addr: SocketAddr,
}

type StatePtr = Arc<RwLock<State>>;

impl Default for State {
    fn default() -> Self {
        // Used by Link: 224.76.78.75:20808
        let ip = Ipv4Addr::new(224, 76, 78, 75).into();
        let port = 30808;
        Self {
            addr: SocketAddr::new(ip, port),
        }
    }
}

// TODO: Use tokio runtime?
// Testing with Bash: echo "foo" > /dev/udp/224.76.78.75/30808
async fn listen(session: StatePtr, init_end: Arc<Barrier>, quit: Arc<AtomicBool>) {
    // TODO: Shouldn't this initialization be inside of original thread to nicely bubble up errors?
    let socket_addr = session.read().unwrap().addr;
    let IpAddr::V4(multicast) = session.read().unwrap().addr.ip() else {
        unreachable!();
    };

    let socket = tokio::net::UdpSocket::bind(socket_addr)
        .await
        .expect("socket creation");

    socket
        .join_multicast_v4(multicast, Ipv4Addr::new(0, 0, 0, 0))
        .expect("joining multicast group");

    info!("listening on {multicast}");
    init_end.wait();

    // TODO: Interruptible loop
    while !quit.load(std::sync::atomic::Ordering::Relaxed) {
        let mut buf = [0u8; 32 * 1024]; // 32 kiB should be enough for everyone
        match socket.recv_from(&mut buf).await {
            Ok((len, remote_addr)) => {
                assert!(remote_addr.is_ipv4());
                let remote_addr = remote_addr.ip();
                let _ = tracing::span!(
                    tracing::Level::INFO,
                    "response",
                    remote = format!("{:?}", remote_addr)
                );
                let incoming = &buf[..len];
                info!(
                    "Received {len} bytes from {remote_addr:?}: {s:?}",
                    s = String::from_utf8_lossy(incoming)
                );
            }
            Err(err) => {
                if err.kind() != std::io::ErrorKind::WouldBlock {
                    tracing::error!("Received error: {err}");
                }
            }
        }
    }
}

#[test]
fn test_defaults() {
    assert!(State::default().addr.ip().is_multicast());
}
