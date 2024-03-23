use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

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

#[derive(Clone)]
struct Connection {
    socket: Arc<tokio::net::UdpSocket>,
    address: SocketAddr,
}

impl Connection {
    fn bind() -> Connection {
        let multicast = Ipv4Addr::new(224, 76, 78, 75);
        let port = 30808;
        let address = SocketAddr::new(IpAddr::V4(multicast), port);

        let socket = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )
        .expect("socket creation");

        socket.set_nonblocking(true).unwrap();
        socket.set_reuse_address(true).unwrap();
        socket.set_multicast_loop_v4(true).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs_f64(0.1)))
            .unwrap();
        socket
            .join_multicast_v4(&multicast, &Ipv4Addr::new(0, 0, 0, 0))
            .unwrap();
        socket.bind(&address.into()).unwrap();

        let socket = tokio::net::UdpSocket::from_std(socket.into()).unwrap();
        let socket = Arc::new(socket);

        Connection { socket, address }
    }

    async fn send(&self, frame: GroupFrame) {
        tracing::debug!("sending packet: {frame}");
        let packet = bincode::serialize(&frame).unwrap();
        self.socket.send_to(&packet, self.address).await.unwrap();
    }

    async fn listen(&self, mut wait_for_cancel: tokio::sync::mpsc::Receiver<()>) {
        tracing::info!("Started linky_groups");
        tracing::debug!("listening on {address}", address = self.address);

        let mut buf = [0_u8; std::mem::size_of::<GroupFrame>()];
        loop {
            let (len, remote_addr) = tokio::select! {
                response = self.socket.recv_from(&mut buf) => response.unwrap(),
                _ = wait_for_cancel.recv() => {
                    tracing::debug!("Recevied shutdown");
                    break;
                },
            };
            let frame: GroupFrame = match bincode::deserialize(&buf[..len]) {
                Ok(v) => v,
                Err(err) => {
                    tracing::error!("Failed to decode bincoded GroupFrame: {err}");
                    continue;
                }
            };
            if !frame.is_supported() {
                tracing::error!("Frame {frame:?} is not supported");
                continue;
            }
            tracing::debug!("From {remote_addr:?}: {frame}");
        }
    }
}

pub struct Groups {
    connection: Connection,

    /// Listening task that receives group messages
    listener: tokio::task::JoinHandle<()>,

    /// Group that is currently playing
    current_group: Option<GroupFrame>,

    /// Channel used to issue cancelation request
    cancel: tokio::sync::mpsc::Sender<()>,
}

impl Future for Groups {
    type Output = <tokio::task::JoinHandle<()> as Future>::Output;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        Pin::new(&mut self.get_mut().listener).poll(cx)
    }
}

#[derive(Debug)]
pub enum Error {
    GroupIdTooLong,
}

impl Groups {
    pub async fn start(&mut self, group_id_str: &str) -> Result<(), Error> {
        let mut group_id: GroupId = Default::default();
        if group_id_str.len() > group_id.len() {
            return Err(Error::GroupIdTooLong);
        }
        group_id[..group_id_str.len()].copy_from_slice(group_id_str.as_bytes());

        let timestamp = 0_i64;
        let frame = GroupFrame::new(group_id, timestamp);
        self.current_group = Some(frame);
        self.connection.send(frame).await;
        Ok(())
    }

    pub fn stop() {
        todo!();
    }

    pub async fn shutdown(self) {
        tracing::debug!("Issuing shutdown");
        self.cancel.send(()).await.unwrap();
        self.listener.await.unwrap();
    }
}

pub fn listen() -> Groups {
    let connection = Connection::bind();

    let (cancel, wait_for_cancel) = tokio::sync::mpsc::channel(1);

    Groups {
        connection: connection.clone(),
        listener: tokio::spawn(async move { connection.listen(wait_for_cancel).await }),
        current_group: None,
        cancel,
    }
}
