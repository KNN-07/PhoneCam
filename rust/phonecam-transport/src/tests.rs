use std::time::{Duration, Instant};

use phonecam_protocol::{Handshake, Message, StatusUpdate, VideoFrame};

use crate::{ConnectionState, PhoneCamClient, PhoneCamServer, TransportConnection};

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

        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for state {expected:?}, current={:?}",
                connection.current_state()
            );
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn make_handshake(name: &str) -> Message {
    Message::Handshake(Handshake {
        version: 1,
        device_name: name.to_string(),
        supported_resolutions: vec![(1920, 1080)],
        supported_fps: vec![30],
    })
}

fn make_frame(size: usize, pts_us: u64, is_keyframe: bool) -> Message {
    Message::VideoFrame(VideoFrame {
        nal_unit: vec![0xAB; size].into(),
        pts_us,
        width: 1920,
        height: 1080,
        is_keyframe,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_lifecycle() {
    let server = PhoneCamServer::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();

    let server_task = tokio::spawn(async move { server.accept().await.unwrap() });
    let client_conn = PhoneCamClient::connect(addr).await.unwrap();
    let mut server_conn = server_task.await.unwrap();

    wait_for_state(
        &client_conn,
        ConnectionState::Streaming,
        Duration::from_secs(2),
    )
    .await;
    wait_for_state(
        &server_conn,
        ConnectionState::Streaming,
        Duration::from_secs(2),
    )
    .await;

    client_conn
        .sender()
        .send(make_handshake("client-manual"))
        .await
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(1), server_conn.receiver().recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(received, Message::Handshake(_)));

    drop(client_conn);
    wait_for_state(
        &server_conn,
        ConnectionState::Disconnected,
        Duration::from_secs(6),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn throughput_1080p30() {
    let server = PhoneCamServer::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();

    let server_task = tokio::spawn(async move { server.accept().await.unwrap() });
    let client_conn = PhoneCamClient::connect(addr).await.unwrap();
    let mut server_conn = server_task.await.unwrap();

    wait_for_state(
        &client_conn,
        ConnectionState::Streaming,
        Duration::from_secs(2),
    )
    .await;
    wait_for_state(
        &server_conn,
        ConnectionState::Streaming,
        Duration::from_secs(2),
    )
    .await;

    let frame_count = 30u64;
    let frame_size = 150 * 1024usize;
    let expected_bytes = frame_count as usize * frame_size;

    let start = Instant::now();
    for i in 0..frame_count {
        client_conn
            .sender()
            .send(make_frame(frame_size, i * 33_333, i == 0))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(33)).await;
    }

    let mut total_bytes = 0usize;
    let mut received_frames = 0u64;
    while received_frames < frame_count {
        let message = tokio::time::timeout(Duration::from_secs(2), server_conn.receiver().recv())
            .await
            .unwrap()
            .unwrap();
        if let Message::VideoFrame(frame) = message {
            total_bytes += frame.nal_unit.len();
            received_frames += 1;
        }
    }

    assert_eq!(total_bytes, expected_bytes);

    let elapsed = start.elapsed().as_secs_f64();
    let mbps = ((total_bytes as f64) * 8.0) / elapsed / 1_000_000.0;
    assert!(mbps >= 5.0, "throughput too low: {mbps:.2} Mbps");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keepalive_timeout() {
    let server = PhoneCamServer::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();

    let accept_task = tokio::spawn(async move { server.accept().await.unwrap() });

    let _silent = tokio::net::TcpStream::connect(addr).await.unwrap();
    let server_conn = accept_task.await.unwrap();

    wait_for_state(
        &server_conn,
        ConnectionState::Disconnected,
        Duration::from_secs(7),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn latency() {
    let server = PhoneCamServer::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();

    let server_task = tokio::spawn(async move {
        let mut conn = server.accept().await.unwrap();
        loop {
            match conn.receiver().recv().await {
                Some(Message::StatusUpdate(update)) if update.status.starts_with("latency:") => {
                    conn.sender()
                        .send(Message::StatusUpdate(StatusUpdate {
                            status: update.status,
                        }))
                        .await
                        .unwrap();
                    break;
                }
                Some(_) => continue,
                None => break,
            }
        }
    });

    let mut client_conn = PhoneCamClient::connect(addr).await.unwrap();
    wait_for_state(
        &client_conn,
        ConnectionState::Streaming,
        Duration::from_secs(2),
    )
    .await;

    let token = format!("latency:{}", std::process::id());
    let send_at = Instant::now();
    client_conn
        .sender()
        .send(Message::StatusUpdate(StatusUpdate {
            status: token.clone(),
        }))
        .await
        .unwrap();

    loop {
        let message = tokio::time::timeout(Duration::from_secs(2), client_conn.receiver().recv())
            .await
            .unwrap()
            .unwrap();
        if let Message::StatusUpdate(update) = message {
            if update.status == token {
                break;
            }
        }
    }

    let rtt = send_at.elapsed();
    assert!(rtt < Duration::from_millis(10), "rtt too high: {rtt:?}");

    server_task.await.unwrap();
}
