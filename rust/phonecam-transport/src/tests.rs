use std::time::{Duration, Instant};

use phonecam_protocol::{
    framing::encode_frame, Handshake, Message, StatusUpdate, StreamProfile, VideoCodec, VideoFrame,
    PROTOCOL_VERSION,
};
use tokio::io::AsyncWriteExt;

use crate::{ConnectionState, PhoneCamClient, PhoneCamServer, TransportConnection, TransportError};

fn h264_1080p30() -> StreamProfile {
    StreamProfile {
        codec: VideoCodec::H264,
        width: 1920,
        height: 1080,
        fps: 30,
    }
}

fn handshake(name: &str, active_profile: Option<StreamProfile>) -> Handshake {
    Handshake {
        version: PROTOCOL_VERSION,
        device_name: name.to_owned(),
        supported_profiles: vec![StreamProfile::H264_720P30, h264_1080p30()],
        active_profile,
    }
}

fn make_frame(size: usize, pts_us: u64, is_keyframe: bool) -> Message {
    Message::VideoFrame(VideoFrame {
        data: vec![0xAB; size].into(),
        pts_us,
        codec: VideoCodec::H264,
        width: 1920,
        height: 1080,
        is_keyframe,
    })
}

async fn wait_for_state(
    connection: &TransportConnection,
    expected: ConnectionState,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        if connection.current_state() == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expected:?}; current={:?}",
            connection.current_state()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn caller_supplied_handshakes_are_exchanged_and_not_polled() {
    let server_handshake = handshake("desktop", None);
    let client_handshake = handshake("phone", Some(h264_1080p30()));
    let server = PhoneCamServer::bind("127.0.0.1:0", server_handshake.clone())
        .await
        .unwrap();
    let addr = server.local_addr().unwrap();
    let server_task = tokio::spawn(async move { server.accept().await.unwrap() });

    let client = PhoneCamClient::connect(addr, client_handshake.clone())
        .await
        .unwrap();
    let mut server = server_task.await.unwrap();
    assert_eq!(client.peer_handshake(), &server_handshake);
    assert_eq!(server.peer_handshake(), &client_handshake);
    assert_eq!(client.current_state(), ConnectionState::Streaming);
    assert_eq!(server.current_state(), ConnectionState::Streaming);

    client
        .sender()
        .send(Message::StatusUpdate(StatusUpdate {
            status: "application-message".to_owned(),
        }))
        .await
        .unwrap();
    let received = tokio::time::timeout(Duration::from_secs(1), server.receiver().recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        received,
        Message::StatusUpdate(StatusUpdate { status }) if status == "application-message"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn protocol_version_mismatch_is_rejected() {
    let server = PhoneCamServer::bind("127.0.0.1:0", handshake("desktop", None))
        .await
        .unwrap();
    let addr = server.local_addr().unwrap();
    let accept = tokio::spawn(async move { server.accept().await });
    let mut raw_peer = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut invalid = handshake("old-phone", Some(StreamProfile::H264_720P30));
    invalid.version = 1;
    raw_peer
        .write_all(&encode_frame(&Message::Handshake(invalid)).unwrap())
        .await
        .unwrap();

    let error = accept.await.unwrap().unwrap_err();
    assert!(matches!(error, TransportError::InvalidHandshake(_)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn active_profile_outside_common_set_is_rejected() {
    let server_handshake = Handshake {
        version: PROTOCOL_VERSION,
        device_name: "desktop".to_owned(),
        supported_profiles: vec![StreamProfile::H264_720P30],
        active_profile: None,
    };
    let client_handshake = Handshake {
        version: PROTOCOL_VERSION,
        device_name: "phone".to_owned(),
        supported_profiles: vec![h264_1080p30()],
        active_profile: Some(h264_1080p30()),
    };
    let server = PhoneCamServer::bind("127.0.0.1:0", server_handshake)
        .await
        .unwrap();
    let addr = server.local_addr().unwrap();
    let accept = tokio::spawn(async move { server.accept().await });

    let client_error = PhoneCamClient::connect(addr, client_handshake)
        .await
        .unwrap_err();
    let server_error = accept.await.unwrap().unwrap_err();
    assert!(matches!(
        client_error,
        TransportError::ActiveProfileNotCommon(_)
    ));
    assert!(matches!(
        server_error,
        TransportError::ActiveProfileNotCommon(_)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn throughput_1080p30() {
    let server = PhoneCamServer::bind("127.0.0.1:0", handshake("desktop", None))
        .await
        .unwrap();
    let addr = server.local_addr().unwrap();
    let server_task = tokio::spawn(async move { server.accept().await.unwrap() });
    let client = PhoneCamClient::connect(addr, handshake("phone", Some(h264_1080p30())))
        .await
        .unwrap();
    let mut server = server_task.await.unwrap();

    let frame_count = 30u64;
    let frame_size = 150 * 1024usize;
    let expected_bytes = frame_count as usize * frame_size;
    let start = Instant::now();
    for index in 0..frame_count {
        client
            .sender()
            .send(make_frame(frame_size, index * 33_333, index == 0))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(33)).await;
    }

    let mut total_bytes = 0usize;
    while total_bytes < expected_bytes {
        let message = tokio::time::timeout(Duration::from_secs(2), server.receiver().recv())
            .await
            .unwrap()
            .unwrap();
        if let Message::VideoFrame(frame) = message {
            total_bytes += frame.data.len();
        }
    }
    assert_eq!(total_bytes, expected_bytes);
    let mbps = ((total_bytes as f64) * 8.0) / start.elapsed().as_secs_f64() / 1_000_000.0;
    assert!(mbps >= 5.0, "throughput too low: {mbps:.2} Mbps");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn established_connection_still_enforces_keepalive_timeout() {
    let server = PhoneCamServer::bind("127.0.0.1:0", handshake("desktop", None))
        .await
        .unwrap();
    let addr = server.local_addr().unwrap();
    let accept = tokio::spawn(async move { server.accept().await.unwrap() });
    let mut raw_peer = tokio::net::TcpStream::connect(addr).await.unwrap();
    raw_peer
        .write_all(
            &encode_frame(&Message::Handshake(handshake(
                "phone",
                Some(StreamProfile::H264_720P30),
            )))
            .unwrap(),
        )
        .await
        .unwrap();
    let connection = accept.await.unwrap();
    wait_for_state(
        &connection,
        ConnectionState::Disconnected,
        Duration::from_secs(7),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn latency() {
    let server = PhoneCamServer::bind("127.0.0.1:0", handshake("desktop", None))
        .await
        .unwrap();
    let addr = server.local_addr().unwrap();
    let server_task = tokio::spawn(async move {
        let mut connection = server.accept().await.unwrap();
        while let Some(message) = connection.receiver().recv().await {
            if let Message::StatusUpdate(update) = message {
                if update.status.starts_with("latency:") {
                    connection
                        .sender()
                        .send(Message::StatusUpdate(update))
                        .await
                        .unwrap();
                    return;
                }
            }
        }
    });
    let mut client =
        PhoneCamClient::connect(addr, handshake("phone", Some(StreamProfile::H264_720P30)))
            .await
            .unwrap();
    let token = format!("latency:{}", std::process::id());
    let send_at = Instant::now();
    client
        .sender()
        .send(Message::StatusUpdate(StatusUpdate {
            status: token.clone(),
        }))
        .await
        .unwrap();
    loop {
        let message = tokio::time::timeout(Duration::from_secs(2), client.receiver().recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(message, Message::StatusUpdate(StatusUpdate { status }) if status == token) {
            break;
        }
    }
    assert!(
        send_at.elapsed() < Duration::from_millis(10),
        "round-trip latency exceeded 10 ms"
    );
    server_task.await.unwrap();
}
