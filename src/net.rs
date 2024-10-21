//! Abstraction over collection of sockets
//!
//! This module is used to simplify interaction with group of sockets.
//! Group of sockets represent all of the IPv4 interfaces that can be binded to
//! and listened on.
// TODO: Support IPv6?
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

/// Collection of references to sockets on all IPv4 interfaces
pub struct Sockets {
    pub sockets: Vec<Arc<tokio::net::UdpSocket>>,
}

impl Sockets {
    /// Bind multicast to all interfaces
    ///
    /// Why bind to all interfaces? From testing binding to 0.0.0.0 will make OS bind to the
    /// gateway interface. For this reason connection from for example host to vm will not work
    pub fn bind() -> Self {
        let sockets: Vec<_> = get_current_ipv4_addresses()
            .into_iter()
            .filter_map(|addr| match open_multicast(addr) {
                Ok(socket) => Some(Arc::new(socket)),
                Err(error) => {
                    tracing::error!(
                        "failed to open multicast socket for interface {addr}: {error}"
                    );
                    None
                }
            })
            .collect();
        assert!(!sockets.is_empty());
        Self { sockets }
    }

    /// Send group frame via all sockets (= all interfaces)
    pub async fn send(&self, frame: crate::GroupFrame) {
        tracing::debug!("sending packet: {frame}");
        let packet = bincode::serialize(&frame).unwrap();

        let target = multicast();

        for socket in &self.sockets {
            // TODO: Don't ignore but ignore socket when it continously fails.
            let _ = socket.send_to(&packet, target).await;
        }
    }

    /// Listen on all interfaces and send incoming packets to negotiator.
    pub async fn listen(
        &self,
        state: tokio::sync::mpsc::Sender<crate::Action>,
        mut wait_for_cancel: tokio::sync::mpsc::Receiver<()>,
    ) {
        tracing::info!("Started linky_groups");

        let (frames_out, mut frames) = tokio::sync::mpsc::channel(4);

        let mut workers = tokio::task::JoinSet::new();

        for socket in &self.sockets {
            let socket = socket.clone();
            let frames_out = frames_out.clone();

            workers.spawn(async move {
                let mut buf = [0u8; std::mem::size_of::<crate::GroupFrame>()];
                loop {
                    // TODO: This may fail for legitimate reasons, so don't just unwrap it.
                    let (len, remote) = socket.recv_from(&mut buf).await.unwrap();
                    let frame: crate::GroupFrame = match bincode::deserialize(&buf[..len]) {
                        Ok(v) => v,
                        Err(err) => {
                            tracing::error!("Failed to decode bincoded GroupFrame: {err}");
                            continue;
                        }
                    };
                    // TODO: Gracefully handle this unwrap
                    frames_out.send((frame, remote)).await.unwrap();
                }
            });
        }

        loop {
            let Some((frame, _remote)) = (tokio::select! {
                response = frames.recv() => response,
                _ = wait_for_cancel.recv() => {
                    tracing::debug!("Recevied shutdown");
                    break;
                },
            }) else {
                break;
            };

            if !frame.is_supported() {
                tracing::error!("Frame {frame:?} is not supported");
                continue;
            }
            state.send(crate::Action::Join(frame)).await.unwrap();
        }
    }
}

/// Get all IPv4 interface addresses on local machine
fn get_current_ipv4_addresses() -> Vec<Ipv4Addr> {
    local_ip_address::list_afinet_netifas()
        .unwrap()
        .iter()
        .map(|(_, address)| *address)
        .filter(|address| address.is_ipv4())
        .map(|address| match address {
            IpAddr::V4(v4) => v4,
            IpAddr::V6(_) => unreachable!(),
        })
        .collect()
}

/// Get multicast address for linky_groups communication
fn multicast() -> std::net::SocketAddr {
    std::net::SocketAddr::new(Ipv4Addr::new(224, 76, 78, 75).into(), 20810)
}

// TODO: Support IPv6
/// Create UDP multicast capable socket for given IPv4 interface.
fn open_multicast(interface: Ipv4Addr) -> std::io::Result<tokio::net::UdpSocket> {
    let multicast = multicast();

    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .expect("socket creation");

    socket.set_multicast_if_v4(&interface)?;
    socket.set_nonblocking(true)?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_read_timeout(Some(std::time::Duration::from_secs_f64(0.1)))?;
    socket.set_multicast_loop_v4(interface.is_loopback())?;

    let IpAddr::V4(address) = multicast.ip() else {
        unreachable!();
    };
    socket.join_multicast_v4(&address, &interface)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, multicast.port()).into())?;
    tracing::info!("Bound interface {interface} to multicast group {multicast}");

    Ok(tokio::net::UdpSocket::from_std(socket.into()).unwrap())
}
