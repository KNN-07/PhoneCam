use std::net::{SocketAddr, ToSocketAddrs};

use phonecam_protocol::Handshake;
use tokio::{net::TcpListener, sync::watch};

use crate::{
    client::{build_connection, TransportConnection, TransportError},
    ConnectionState,
};

pub struct PhoneCamServer {
    listener: TcpListener,
    local_handshake: Handshake,
}

impl PhoneCamServer {
    pub async fn bind<A: ToSocketAddrs>(
        addr: A,
        local_handshake: Handshake,
    ) -> Result<Self, TransportError> {
        local_handshake.validate()?;
        let mut addrs = addr.to_socket_addrs()?;
        let target = addrs.next().ok_or_else(|| {
            TransportError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "no socket address resolved",
            ))
        })?;

        let listener = TcpListener::bind(target).await?;
        Ok(Self {
            listener,
            local_handshake,
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        Ok(self.listener.local_addr()?)
    }

    pub async fn accept(&self) -> Result<TransportConnection, TransportError> {
        let (stream, _) = self.listener.accept().await?;
        stream.set_nodelay(true)?;

        let (state_tx, state_rx) = watch::channel(ConnectionState::Handshaking);
        build_connection(stream, state_tx, state_rx, self.local_handshake.clone()).await
    }
}
