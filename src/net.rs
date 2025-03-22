//! Abstraction over collection of sockets
//!
//! This module is used to simplify interaction with group of sockets.
//! Group of sockets represent all of the IPv4 interfaces that can be binded to
//! and listened on.
// TODO: Support IPv6?
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

/// Collection of references to sockets on all IPv4 interfaces
#[derive(Default)]
pub struct Connection {
    /// Sockets bound for each network interface
    pub sockets: Vec<(Ipv4Addr, Arc<tokio::net::UdpSocket>)>,

    /// Addresses for which we have established working socket
    pub established: HashSet<Ipv4Addr>,

    /// Addresses for which we failed to construct a socket (but we will keep trying in next
    /// refres)
    pub rejected: HashSet<Ipv4Addr>,

    /// Set of workers that listen on given connections (1 per socket)
    pub workers: tokio::task::JoinSet<()>,
}

impl Connection {
    /// Bind multicast to all interfaces
    ///
    /// Why bind to all interfaces? From testing binding to 0.0.0.0 will make OS bind to the
    /// gateway interface. For this reason connection from for example host to vm will not work
    fn bind(
        &mut self,
        frames_out: tokio::sync::mpsc::Sender<(crate::GroupFrame, std::net::SocketAddr)>,
    ) {
        /// How many times we can fail to establish multicast connection
        ///
        /// (probably unnesesary mechanism I don't remember why I introduced it)
        const MAX_TRIES: i32 = 5;

        'next_address: for addr in get_current_ipv4_addresses() {
            if self.established.contains(&addr) {
                continue;
            }

            let mut tries = 0;
            loop {
                match open_multicast(addr) {
                    Ok(socket) => {
                        self.established.insert(addr);
                        let socket = Arc::new(socket);
                        self.sockets.push((addr, socket.clone()));
                        let frames_out = frames_out.clone();

                        // TODO: This may be potential infinite growth in a network that constantly
                        // looses connection
                        self.workers.spawn(async move {
                            let mut buf = [0u8; std::mem::size_of::<crate::GroupFrame>()];
                            loop {
                                // TODO: This may fail for legitimate reasons, so don't just unwrap it.
                                let (len, remote) = socket.recv_from(&mut buf).await.unwrap();
                                let frame: crate::GroupFrame =
                                    match bincode::deserialize(&buf[..len]) {
                                        Ok(v) => v,
                                        Err(err) => {
                                            tracing::error!(
                                                "Failed to decode bincoded GroupFrame: {err}"
                                            );
                                            continue;
                                        }
                                    };
                                // TODO: Gracefully handle this unwrap
                                frames_out.send((frame, remote)).await.unwrap();
                            }
                        });
                        continue 'next_address;
                    }
                    Err(err) => {
                        tries += 1;
                        if tries > MAX_TRIES {
                            if self.rejected.insert(addr) {
                                tracing::warn!(
                                    "failed to open multicast socket for interface {addr}: {err}"
                                );
                            }
                            continue 'next_address;
                        }
                    }
                }
            }
        }

        assert!(
            !self.sockets.is_empty(),
            "Cannot use Harmonia without a network over which we could synchronize!"
        );
    }

    /// Remove a set of ids that identify sockets from `sockets` field.
    ///
    /// Useful when some connections repeatedly fail to work
    fn remove(&mut self, ids: Vec<usize>) {
        assert!(!ids.is_empty());
        assert!(ids.iter().is_sorted());

        for id in ids.iter().rev() {
            let (addr, _socket) = self.sockets.swap_remove(*id);
            self.established.remove(&addr);
        }
    }
}

/// Listen on all interfaces and send incoming packets to negotiator.
pub async fn listen(
    mut recv_frame: tokio::sync::mpsc::Receiver<crate::GroupFrame>,
    state: tokio::sync::mpsc::Sender<crate::Action>,
    mut wait_for_cancel: tokio::sync::mpsc::Receiver<()>,
    is_enabled: bool,
) {
    if !is_enabled {
        tracing::info!("Skipping linky_groups start since Link is disabled");
        return;
    }
    tracing::info!("Started linky_groups");

    let (frames_out, mut frames) = tokio::sync::mpsc::channel(4);

    let mut connection: Connection = Default::default();
    connection.bind(frames_out.clone());

    let target_addr = multicast();

    let mut rebind_multicast = tokio::time::interval(std::time::Duration::from_secs(5));

    let mut failures = std::collections::HashMap::new();

    'worker: loop {
        tokio::select! {
            Some((frame, _remote)) = frames.recv() => {
                if !frame.is_supported() {
                    tracing::error!("Frame {frame:?} is not supported");
                    continue;
                }
                state.send(crate::Action::Join(frame)).await.unwrap();
            }

            Some(frame) = recv_frame.recv() => {
                tracing::debug!("sending packet: {frame}");
                let packet = bincode::serialize(&frame).unwrap();

                let mut failed_sockets = vec![];

                for (id, (addr, socket)) in connection.sockets.iter().enumerate() {
                    if let Err(err) = socket.send_to(&packet, target_addr).await {
                        let recorded_failures = *failures.get(addr).unwrap_or(&0);
                        if recorded_failures > 5 {
                            tracing::error!("Failed sending 5 times in the row, removing socket on interface {addr}: {err}");
                            failed_sockets.push(id);
                        }
                        failures.insert(*addr, recorded_failures+1);

                        tracing::warn!("Failed to send via socket on interface {addr}: {err}");
                    } else {
                        failures.remove(addr);
                    }
                }

                if !failed_sockets.is_empty() {
                    connection.remove(failed_sockets);
                }
            }

            _ = rebind_multicast.tick() => {
                connection.bind(frames_out.clone());
            }


            _ = wait_for_cancel.recv() => {
                tracing::debug!("Recevied shutdown");
                break 'worker
            }

            else => break 'worker,
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
