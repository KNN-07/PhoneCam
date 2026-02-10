use std::{net::ToSocketAddrs, time::Duration};

use phonecam_protocol::{
    framing::{decode_frame, encode_frame, FrameError, FRAME_LENGTH_PREFIX_BYTES},
    Handshake, Message, StatusUpdate,
};
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{tcp::OwnedReadHalf, tcp::OwnedWriteHalf, TcpStream},
    sync::{mpsc, watch},
    time::{interval, timeout, MissedTickBehavior},
};

use crate::ConnectionState;

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(1);
const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(5);
const OUTBOUND_CHANNEL_CAPACITY: usize = 8;
const INBOUND_CHANNEL_CAPACITY: usize = 64;
const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
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
}

#[derive(Debug)]
pub struct TransportConnection {
    sender: mpsc::Sender<Message>,
    receiver: mpsc::Receiver<Message>,
    state_rx: watch::Receiver<ConnectionState>,
    _lifecycle_worker: tokio::task::JoinHandle<()>,
}

impl TransportConnection {
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
    pub async fn connect<A: ToSocketAddrs>(addr: A) -> Result<TransportConnection, TransportError> {
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

        build_connection(stream, state_tx, state_rx, "client").await
    }
}

pub(crate) async fn build_connection(
    stream: TcpStream,
    state_tx: watch::Sender<ConnectionState>,
    state_rx: watch::Receiver<ConnectionState>,
    role_name: &str,
) -> Result<TransportConnection, TransportError> {
    let (outbound_tx, outbound_rx) = mpsc::channel::<Message>(OUTBOUND_CHANNEL_CAPACITY);
    let (inbound_tx, inbound_rx) = mpsc::channel::<Message>(INBOUND_CHANNEL_CAPACITY);

    let (read_half, write_half) = stream.into_split();

    let writer_handle = tokio::spawn(writer_loop(write_half, outbound_rx));
    let reader_handle = tokio::spawn(reader_loop(
        read_half,
        inbound_tx,
        state_tx.clone(),
    ));

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

    outbound_tx
        .send(default_handshake(role_name))
        .await
        .map_err(|_| TransportError::SenderClosed)?;

    Ok(TransportConnection {
        sender: outbound_tx,
        receiver: inbound_rx,
        state_rx,
        _lifecycle_worker: lifecycle,
    })
}

async fn writer_loop(
    mut write_half: OwnedWriteHalf,
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
    mut read_half: OwnedReadHalf,
    inbound_tx: mpsc::Sender<Message>,
    state_tx: watch::Sender<ConnectionState>,
) -> Result<(), TransportError> {
    loop {
        let message = match timeout(KEEPALIVE_TIMEOUT, read_message(&mut read_half)).await {
            Ok(message_result) => message_result?,
            Err(_) => return Err(TransportError::KeepaliveTimeout(KEEPALIVE_TIMEOUT)),
        };

        match &message {
            Message::Handshake(_) => {
                let _ = state_tx.send(ConnectionState::Streaming);
            }
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

async fn write_message(write_half: &mut OwnedWriteHalf, message: &Message) -> Result<(), TransportError> {
    let encoded = encode_frame(message)?;
    write_half.write_all(&encoded).await?;
    Ok(())
}

async fn read_message(read_half: &mut OwnedReadHalf) -> Result<Message, TransportError> {
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

fn default_handshake(role_name: &str) -> Message {
    Message::Handshake(Handshake {
        version: 1,
        device_name: format!("phonecam-{role_name}"),
        supported_resolutions: vec![(1920, 1080)],
        supported_fps: vec![30],
    })
}
