use std::{
    mem::MaybeUninit,
    net::{Ipv4Addr, SocketAddr},
    sync::{atomic::AtomicBool, Arc, Barrier, RwLock},
    thread::JoinHandle,
    time::Duration,
};

use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tracing::info;

pub struct StartListSession {
    #[allow(dead_code)]
    state: StatePtr,
    quit: Arc<AtomicBool>,
    listener: Option<JoinHandle<()>>,
}

impl Drop for StartListSession {
    fn drop(&mut self) {
        self.quit.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(listener) = self.listener.take() {
            listener.join().unwrap();
        }
    }
}

impl StartListSession {
    pub fn start_listening() -> Self {
        let state = Arc::new(RwLock::new(Default::default()));
        let init_end = Arc::new(Barrier::new(2));
        let quit = Arc::new(AtomicBool::new(false));
        let mine_init_end = init_end.clone();

        let session = Self {
            state: state.clone(),
            quit: quit.clone(),
            listener: Some(
                std::thread::Builder::new()
                    .name("linky_start_listener".to_string())
                    .spawn(move || listen(state, init_end, quit))
                    .expect("initialization of listener"),
            ),
        };

        // Wait so listener can setup all that it needs
        mine_init_end.wait();
        session
    }
}

struct State {
    addr: SockAddr,
}

type StatePtr = Arc<RwLock<State>>;

impl Default for State {
    fn default() -> Self {
        // Used by Link: 224.76.78.75:20808
        let ip = Ipv4Addr::new(224, 76, 78, 75).into();
        let port = 30808;
        Self {
            addr: SocketAddr::new(ip, port).into(),
        }
    }
}

// TODO: Use tokio runtime?
// Testing with Bash: echo "foo" > /dev/udp/224.76.78.75/30808
fn listen(session: StatePtr, init_end: Arc<Barrier>, quit: Arc<AtomicBool>) {
    let _log = tracing::span!(tracing::Level::INFO, "liblinkystart").entered();

    // TODO: Shouldn't this initialization be inside of original thread to nicely bubble up errors?
    let session_locked = session.read().unwrap();

    let socket =
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).expect("socket creation");

    socket
        .set_read_timeout(Some(Duration::from_millis(50)))
        .expect("setting socket read timeout");

    let multicast = session_locked.addr.as_socket_ipv4().unwrap();
    socket
        .join_multicast_v4(multicast.ip(), &Ipv4Addr::new(0, 0, 0, 0))
        // TODO: Why this could fail?
        .expect("joining multicast");

    socket.bind(&session_locked.addr).expect("socket bind");

    info!("listening on {multicast}");

    init_end.wait();

    // TODO: Interruptible loop
    while !quit.load(std::sync::atomic::Ordering::Relaxed) {
        let mut buf = [0u8; 32 * 1024]; // 32 kiB should be enough for everyone
                                        // Stolen from https://docs.rs/socket2/latest/aarch64-linux-android/src/socket2/socket.rs.html#2042
                                        // For current Rust stable I think this is best solution
                                        // In the future this https://doc.rust-lang.org/stable/std/mem/union.MaybeUninit.html#method.slice_assume_init_mut
                                        // probably should be used
        let buf_uninit = unsafe { &mut *(&mut buf as *mut [u8] as *mut [MaybeUninit<u8>]) };
        match socket.recv_from(buf_uninit) {
            Ok((len, remote_addr)) => {
                assert!(remote_addr.is_ipv4());
                let remote_addr = remote_addr.as_socket_ipv4().unwrap();
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
    let def = State::default();
    let ip = def.addr.as_socket_ipv4().unwrap();
    assert!(ip.ip().is_multicast());
}
