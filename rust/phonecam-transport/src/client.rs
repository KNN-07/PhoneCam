use std::{net::ToSocketAddrs, time::Duration};

use phonecam_protocol::{
    framing::{decode_frame, encode_frame, FrameError, FRAME_LENGTH_PREFIX_BYTES, MAX_FRAME_BYTES},
    Handshake, Message, ProfileValidationError, StatusUpdate, StreamProfile,
};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::{mpsc, watch},
    time::{interval, timeout, MissedTickBehavior},
};

use crate::ConnectionState;

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(1);
const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(5);
const OUTBOUND_CHANNEL_CAPACITY: usize = 8;
const INBOUND_CHANNEL_CAPACITY: usize = 64;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const KEEPALIVE_PING: &str = "__phonecam_ping__";

#[derive(Debug, Error)]
pub enum TransportError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error("keepalive timeout after {0:?}")]
    KeepaliveTimeout(Duration),
    #[error("frame too large: {0} bytes")]
    FrameTooLarge(usize),
    #[error("connection sender channel closed")]
    SenderClosed,
    #[error(transparent)]
    InvalidHandshake(#[from] ProfileValidationError),
    #[error("handshake timed out after {0:?}")]
    HandshakeTimeout(Duration),
    #[error("expected handshake as the first peer message")]
    ExpectedHandshake,
    #[error("active stream profile is not common to both peers: {0:?}")]
    ActiveProfileNotCommon(StreamProfile),
    #[error("unexpected handshake after the connection was established")]
    UnexpectedHandshake,
}

#[derive(Debug)]
pub struct TransportConnection {
    sender: mpsc::Sender<Message>,
    receiver: mpsc::Receiver<Message>,
    state_rx: watch::Receiver<ConnectionState>,
    peer_handshake: Handshake,
    _lifecycle_worker: tokio::task::JoinHandle<()>,
}

impl TransportConnection {
    pub fn peer_handshake(&self) -> &Handshake {
        &self.peer_handshake
    }
    pub fn sender(&self) -> &mpsc::Sender<Message> {
        &self.sender
    }

    pub fn receiver(&mut self) -> &mut mpsc::Receiver<Message> {
        &mut self.receiver
    }

    pub fn current_state(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }

    pub fn subscribe_state(&self) -> watch::Receiver<ConnectionState> {
        self.state_rx.clone()
    }
}

pub struct PhoneCamClient;

impl PhoneCamClient {
    pub async fn connect<A: ToSocketAddrs>(
        addr: A,
        local_handshake: Handshake,
    ) -> Result<TransportConnection, TransportError> {
        local_handshake.validate()?;
        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        let _ = state_tx.send(ConnectionState::Connecting);

        let mut addrs = addr.to_socket_addrs()?;
        let target = addrs.next().ok_or_else(|| {
            TransportError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "no socket address resolved",
            ))
        })?;

        let stream = TcpStream::connect(target).await?;
        stream.set_nodelay(true)?;
        let _ = state_tx.send(ConnectionState::Handshaking);

        build_connection(stream, state_tx, state_rx, local_handshake).await
    }
}

pub(crate) async fn build_connection(
    mut stream: TcpStream,
    state_tx: watch::Sender<ConnectionState>,
    state_rx: watch::Receiver<ConnectionState>,
    local_handshake: Handshake,
) -> Result<TransportConnection, TransportError> {
    local_handshake.validate()?;
    let peer_handshake = timeout(HANDSHAKE_TIMEOUT, async {
        write_message(&mut stream, &Message::Handshake(local_handshake.clone())).await?;
        match read_message(&mut stream).await? {
            Message::Handshake(handshake) => Ok(handshake),
            _ => Err(TransportError::ExpectedHandshake),
        }
    })
    .await
    .map_err(|_| TransportError::HandshakeTimeout(HANDSHAKE_TIMEOUT))??;
    peer_handshake.validate()?;
    validate_common_active_profiles(&local_handshake, &peer_handshake)?;

    let (outbound_tx, outbound_rx) = mpsc::channel::<Message>(OUTBOUND_CHANNEL_CAPACITY);
    let (inbound_tx, inbound_rx) = mpsc::channel::<Message>(INBOUND_CHANNEL_CAPACITY);
    let (read_half, write_half) = stream.into_split();
    let writer_handle = tokio::spawn(writer_loop(write_half, outbound_rx));
    let reader_handle = tokio::spawn(reader_loop(read_half, inbound_tx));
    let _ = state_tx.send(ConnectionState::Streaming);

    let lifecycle = tokio::spawn(async move {
        tokio::pin!(writer_handle);
        tokio::pin!(reader_handle);

        tokio::select! {
            _ = &mut writer_handle => {
                reader_handle.abort();
                let _ = reader_handle.await;
            }
            _ = &mut reader_handle => {
                writer_handle.abort();
                let _ = writer_handle.await;
            }
        }

        let _ = state_tx.send(ConnectionState::Disconnected);
    });

    Ok(TransportConnection {
        sender: outbound_tx,
        receiver: inbound_rx,
        state_rx,
        peer_handshake,
        _lifecycle_worker: lifecycle,
    })
}

fn validate_common_active_profiles(
    local: &Handshake,
    peer: &Handshake,
) -> Result<(), TransportError> {
    for (active, supported_by_other) in [
        (local.active_profile, peer.supported_profiles.as_slice()),
        (peer.active_profile, local.supported_profiles.as_slice()),
    ] {
        if let Some(active) = active {
            if !supported_by_other.contains(&active) {
                return Err(TransportError::ActiveProfileNotCommon(active));
            }
        }
    }
    Ok(())
}

async fn writer_loop(
    mut write_half: tokio::net::tcp::OwnedWriteHalf,
    mut outbound_rx: mpsc::Receiver<Message>,
) -> Result<(), TransportError> {
    let mut keepalive = interval(KEEPALIVE_INTERVAL);
    keepalive.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            maybe_message = outbound_rx.recv() => {
                match maybe_message {
                    Some(message) => write_message(&mut write_half, &message).await?,
                    None => return Ok(()),
                }
            }
            _ = keepalive.tick() => {
                write_message(
                    &mut write_half,
                    &Message::StatusUpdate(StatusUpdate { status: KEEPALIVE_PING.to_string() }),
                ).await?;
            }
        }
    }
}

async fn reader_loop(
    mut read_half: tokio::net::tcp::OwnedReadHalf,
    inbound_tx: mpsc::Sender<Message>,
) -> Result<(), TransportError> {
    loop {
        let message = match timeout(KEEPALIVE_TIMEOUT, read_message(&mut read_half)).await {
            Ok(message_result) => message_result?,
            Err(_) => return Err(TransportError::KeepaliveTimeout(KEEPALIVE_TIMEOUT)),
        };

        match &message {
            Message::Handshake(_) => return Err(TransportError::UnexpectedHandshake),
            Message::StatusUpdate(status) if status.status == KEEPALIVE_PING => {
                continue;
            }
            _ => {}
        }

        if inbound_tx.send(message).await.is_err() {
            return Ok(());
        }
    }
}

async fn write_message<W>(write_half: &mut W, message: &Message) -> Result<(), TransportError>
where
    W: AsyncWrite + Unpin,
{
    let encoded = encode_frame(message)?;
    write_half.write_all(&encoded).await?;
    Ok(())
}

async fn read_message<R>(read_half: &mut R) -> Result<Message, TransportError>
where
    R: AsyncRead + Unpin,
{
    let mut len_bytes = [0u8; FRAME_LENGTH_PREFIX_BYTES];
    read_half.read_exact(&mut len_bytes).await?;

    let payload_len = u32::from_be_bytes(len_bytes) as usize;
    if payload_len > MAX_FRAME_BYTES {
        return Err(TransportError::FrameTooLarge(payload_len));
    }

    let mut frame = Vec::with_capacity(FRAME_LENGTH_PREFIX_BYTES + payload_len);
    frame.extend_from_slice(&len_bytes);
    frame.resize(FRAME_LENGTH_PREFIX_BYTES + payload_len, 0);
    read_half
        .read_exact(&mut frame[FRAME_LENGTH_PREFIX_BYTES..])
        .await?;

    Ok(decode_frame(&frame)?)
}
