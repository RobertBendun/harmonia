use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

pub struct Sockets {
    pub sockets: Vec<Arc<tokio::net::UdpSocket>>,
}

impl Sockets {
    /// Bind multicast to all interfaces
    ///
    /// Why bind to all interfaces? From testing binding to 0.0.0.0 will make OS bind to the
    /// gateway interface. For this reason connection from for example host to vm will not work
    pub fn bind() -> Self {
        Self {
            sockets: get_current_ipv4_addresses()
                .into_iter()
                .map(|addr| Arc::new(open_multicast(addr)))
                .collect(),
        }
    }

    pub async fn send(&self, frame: crate::GroupFrame) {
        tracing::debug!("sending packet: {frame}");
        let packet = bincode::serialize(&frame).unwrap();

        let target = multicast();

        for socket in &self.sockets {
            // TODO: Don't unwrap here, it may fail for good reasons
            socket.send_to(&packet, target).await.unwrap();
        }
    }

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
                    let frame: crate::GroupFrame = match bincode::deserialize(&mut buf[..len]) {
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

fn multicast() -> std::net::SocketAddr {
    std::net::SocketAddr::new(Ipv4Addr::new(224, 76, 78, 75).into(), 20810)
}

// TODO: Support IPv6
fn open_multicast(interface: Ipv4Addr) -> tokio::net::UdpSocket {
    let multicast = multicast();

    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .expect("socket creation");

    socket.set_multicast_if_v4(&interface).unwrap();
    socket.set_nonblocking(true).unwrap();
    socket.set_reuse_address(true).unwrap();
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs_f64(0.1)))
        .unwrap();
    socket
        .set_multicast_loop_v4(interface.is_loopback())
        .unwrap();

    let IpAddr::V4(address) = multicast.ip() else {
        unreachable!();
    };
    socket.join_multicast_v4(&address, &interface).unwrap();
    socket
        .bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, multicast.port()).into())
        .unwrap();

    tokio::net::UdpSocket::from_std(socket.into()).unwrap()
}
