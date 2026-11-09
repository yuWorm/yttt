use std::time::Duration;

use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf, duplex, split};
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::{
    ClientRequest, HostEvent, Request, ServerEvent, encode_message,
    terminal::{TerminalInput, TerminalMutationContext},
};

use super::*;

fn ping(request_id: u64) -> ControlMessage {
    ControlMessage::Request(ClientRequest::new(
        request_id,
        Request::Ping {
            sent_millis: request_id,
        },
    ))
}

fn event(host_sequence: u64) -> ControlMessage {
    ControlMessage::Event(HostEvent {
        host_sequence,
        body: ServerEvent::ResourceCatalogChanged,
    })
}

fn input(bytes: usize) -> ControlMessage {
    ControlMessage::TerminalInput {
        input: TerminalInput {
            session_id: TerminalSessionId::new("terminal"),
            context: TerminalMutationContext {
                host_epoch: 1,
                session_epoch: 1,
                lease_epoch: 1,
                client_sequence: 1,
                geometry_epoch: 1,
            },
            bytes: vec![b'x'; bytes],
        },
    }
}

fn large_request(request_id: u64, bytes: usize) -> ControlMessage {
    let ControlMessage::TerminalInput { input, .. } = input(bytes) else {
        unreachable!("input builds terminal input");
    };
    ControlMessage::Request(ClientRequest::new(
        request_id,
        Request::TerminalInput(input),
    ))
}

fn render_frame(host_sequence: u64) -> Arc<[u8]> {
    Arc::from(encode_message(FrameKind::Control, &event(host_sequence)).unwrap())
}

/// Sends `message` and lets the writer start it; the one-byte pipe keeps the
/// writer inside that message until the test reads.
async fn occupy_writer(writer: &SessionWriter, message: &ControlMessage) {
    writer.send_reliable(message).unwrap();
    tokio::task::yield_now().await;
}

/// A writer over a small pipe: a frame completes only as the test reads it.
fn paused_pair(
    capacity: usize,
) -> (
    SessionWriter,
    SessionWriterTask,
    SessionReader<ReadHalf<DuplexStream>>,
    WriteHalf<DuplexStream>,
) {
    let (local, peer) = duplex(capacity);
    let (_, write) = split(local);
    let (writer, task) = SessionWriter::spawn(write);
    let (read, peer_write) = split(peer);
    (writer, task, SessionReader::new(read), peer_write)
}

async fn next(reader: &mut SessionReader<ReadHalf<DuplexStream>>) -> ControlMessage {
    tokio::time::timeout(Duration::from_secs(5), reader.receive())
        .await
        .expect("message arrives")
        .unwrap()
}

fn sequence(message: &ControlMessage) -> u64 {
    match message {
        ControlMessage::Event(event) => event.host_sequence,
        other => panic!("expected an event, got {other:?}"),
    }
}

#[tokio::test]
async fn interactive_then_reliable_then_render_once_the_writer_is_free() {
    let (writer, _task, mut reader, _peer) = paused_pair(1);
    let lane = writer.open_render_lane(None);
    occupy_writer(&writer, &event(1)).await;
    lane.enqueue_frame(render_frame(100)).unwrap();
    writer.send_reliable(&event(2)).unwrap();
    writer.send_interactive(&ping(7)).unwrap();

    assert_eq!(sequence(&next(&mut reader).await), 1);
    assert_eq!(next(&mut reader).await, ping(7));
    assert_eq!(sequence(&next(&mut reader).await), 2);
    assert_eq!(sequence(&next(&mut reader).await), 100);
    lane.drain().await.unwrap();
    assert!(lane.is_idle());
}

#[tokio::test]
async fn interactive_messages_pass_between_the_chunks_of_a_large_message() {
    let (writer, _task, mut reader, _peer) = paused_pair(4096);
    let large = large_request(9, 2 * CHUNK_PAYLOAD_BYTES);
    occupy_writer(&writer, &large).await;
    writer.send_reliable(&event(2)).unwrap();
    writer.send_interactive(&input(4)).unwrap();

    assert_eq!(next(&mut reader).await, input(4));
    assert_eq!(next(&mut reader).await, large);
    assert_eq!(sequence(&next(&mut reader).await), 2);
}

#[tokio::test]
async fn a_lane_barrier_writes_queued_frames_before_the_reliable_message() {
    let (writer, _task, mut reader, _peer) = paused_pair(1);
    let lane = writer.open_render_lane(None);
    occupy_writer(&writer, &event(1)).await;
    lane.enqueue_frame(render_frame(100)).unwrap();
    writer.send_reliable_after(&lane, &event(2)).unwrap();

    assert_eq!(sequence(&next(&mut reader).await), 1);
    assert_eq!(sequence(&next(&mut reader).await), 100);
    assert_eq!(sequence(&next(&mut reader).await), 2);
}

#[tokio::test]
async fn render_lanes_take_turns() {
    let (writer, _task, mut reader, _peer) = paused_pair(1);
    let first = writer.open_render_lane(None);
    let second = writer.open_render_lane(None);
    occupy_writer(&writer, &event(1)).await;
    first.enqueue_frame(render_frame(10)).unwrap();
    first.enqueue_frame(render_frame(11)).unwrap();
    second.enqueue_frame(render_frame(20)).unwrap();

    let order = [
        sequence(&next(&mut reader).await),
        sequence(&next(&mut reader).await),
        sequence(&next(&mut reader).await),
        sequence(&next(&mut reader).await),
    ];
    assert_eq!(order, [1, 10, 20, 11]);
}

#[tokio::test]
async fn a_full_render_lane_refuses_frames_without_closing_the_connection() {
    let (writer, _task, mut reader, _peer) = paused_pair(1);
    let lane = writer.open_render_lane(None);
    occupy_writer(&writer, &event(1)).await;
    let big = Arc::<[u8]>::from(
        encode_message(FrameKind::Control, &input(RENDER_LANE_BYTES - 16)).unwrap(),
    );
    lane.enqueue_frame(big).unwrap();
    assert!(lane.enqueue_frame(render_frame(2)).is_err());
    writer.send_reliable(&event(3)).unwrap();
    assert_eq!(sequence(&next(&mut reader).await), 1);
    assert_eq!(sequence(&next(&mut reader).await), 3);
}

#[tokio::test]
async fn reliable_overflow_closes_the_connection() {
    let (writer, _task, _reader, _peer) = paused_pair(1);
    let mut result = Ok(());
    for sequence in 0..=RELIABLE_ITEMS as u64 + 1 {
        result = writer.send_reliable(&event(sequence));
        if result.is_err() {
            break;
        }
    }
    assert!(matches!(result, Err(SessionSendError::Overflow)));
    tokio::time::timeout(Duration::from_secs(1), writer.closed())
        .await
        .unwrap();
    assert_eq!(
        writer.close_reason(),
        Some(SessionCloseReason::Overflow(QueueOverflow::ReliableItems))
    );
    assert!(matches!(
        writer.send_interactive(&ping(1)),
        Err(SessionSendError::Closed)
    ));
}

#[tokio::test]
async fn a_newer_keyed_message_takes_the_place_of_one_still_waiting() {
    let (writer, _task, mut reader, _peer) = paused_pair(1);
    occupy_writer(&writer, &event(1)).await;
    writer.send_reliable_latest("a", &event(2)).unwrap();
    writer.send_reliable(&event(3)).unwrap();
    writer.send_reliable_latest("a", &event(4)).unwrap();
    writer.send_reliable_latest("b", &event(5)).unwrap();
    writer.send_reliable_latest("a", &event(6)).unwrap();

    let order = [
        sequence(&next(&mut reader).await),
        sequence(&next(&mut reader).await),
        sequence(&next(&mut reader).await),
        sequence(&next(&mut reader).await),
    ];
    assert_eq!(order, [1, 6, 3, 5]);
}

#[tokio::test]
async fn a_keyed_message_already_being_written_is_not_replaced() {
    let (writer, _task, mut reader, _peer) = paused_pair(1);
    writer.send_reliable_latest("a", &event(1)).unwrap();
    tokio::task::yield_now().await;
    writer.send_reliable_latest("a", &event(2)).unwrap();

    assert_eq!(sequence(&next(&mut reader).await), 1);
    assert_eq!(sequence(&next(&mut reader).await), 2);
}

#[tokio::test]
async fn repeated_keyed_messages_to_a_stalled_peer_do_not_overflow() {
    let (writer, _task, mut reader, _peer) = paused_pair(1);
    occupy_writer(&writer, &event(0)).await;
    for sequence in 1..=4 * RELIABLE_ITEMS as u64 {
        writer.send_reliable_latest("a", &event(sequence)).unwrap();
    }
    writer.send_reliable(&event(u64::MAX)).unwrap();

    assert_eq!(sequence(&next(&mut reader).await), 0);
    assert_eq!(
        sequence(&next(&mut reader).await),
        4 * RELIABLE_ITEMS as u64
    );
    assert_eq!(sequence(&next(&mut reader).await), u64::MAX);
}

#[tokio::test]
async fn a_render_frame_gets_a_turn_in_a_steady_reliable_stream() {
    let (writer, _task, mut reader, _peer) = paused_pair(1);
    let lane = writer.open_render_lane(None);
    let first = large_request(1, RELIABLE_BURST_BYTES);
    occupy_writer(&writer, &first).await;
    lane.enqueue_frame(render_frame(100)).unwrap();
    writer.send_reliable(&event(2)).unwrap();
    writer.send_reliable(&event(3)).unwrap();

    assert_eq!(next(&mut reader).await, first);
    assert_eq!(sequence(&next(&mut reader).await), 100);
    assert_eq!(sequence(&next(&mut reader).await), 2);
    assert_eq!(sequence(&next(&mut reader).await), 3);
}

#[tokio::test]
async fn a_render_frame_never_overtakes_a_reliable_message_queued_before_it() {
    let (writer, _task, mut reader, _peer) = paused_pair(1);
    let lane = writer.open_render_lane(None);
    let first = large_request(1, RELIABLE_BURST_BYTES);
    occupy_writer(&writer, &first).await;
    writer.send_reliable(&event(2)).unwrap();
    lane.enqueue_frame(render_frame(100)).unwrap();
    writer.send_reliable(&event(3)).unwrap();

    assert_eq!(next(&mut reader).await, first);
    assert_eq!(sequence(&next(&mut reader).await), 2);
    assert_eq!(sequence(&next(&mut reader).await), 100);
    assert_eq!(sequence(&next(&mut reader).await), 3);
}

#[tokio::test]
async fn a_keyed_message_is_waiting_until_its_writing_starts() {
    let (writer, _task, mut reader, _peer) = paused_pair(1);
    occupy_writer(&writer, &event(0)).await;
    assert!(!writer.is_waiting("a"));
    writer.send_reliable_latest("a", &event(1)).unwrap();
    assert!(writer.is_waiting("a"));

    assert_eq!(sequence(&next(&mut reader).await), 0);
    assert_eq!(sequence(&next(&mut reader).await), 1);
    assert!(!writer.is_waiting("a"));
}

#[tokio::test(start_paused = true)]
async fn queue_stats_show_what_waits_and_for_how_long() {
    let (writer, _task, mut reader, _peer) = paused_pair(1);
    occupy_writer(&writer, &event(0)).await;
    writer.send_reliable(&event(1)).unwrap();
    writer.send_reliable(&event(2)).unwrap();
    writer.send_interactive(&ping(3)).unwrap();
    tokio::time::advance(Duration::from_secs(3)).await;

    let stats = writer.queue_stats();
    assert_eq!(stats.interactive_items, 1);
    assert_eq!(stats.reliable_items, 2);
    assert!(stats.reliable_bytes > 0);
    assert_eq!(stats.reliable_items_high_water, 2);
    assert!(stats.oldest_reliable_wait.unwrap() >= Duration::from_secs(3));

    for _ in 0..4 {
        next(&mut reader).await;
    }
    let stats = writer.queue_stats();
    assert_eq!((stats.reliable_items, stats.reliable_bytes), (0, 0));
    assert_eq!(stats.oldest_reliable_wait, None);
    assert_eq!(stats.reliable_items_high_water, 2);
}

fn compressible(request_id: u64) -> ControlMessage {
    large_request(request_id, 4 * CHUNK_PAYLOAD_BYTES)
}

fn encoded(message: &ControlMessage) -> Arc<[u8]> {
    Arc::from(encode_message(FrameKind::Control, message).unwrap())
}

#[tokio::test]
async fn a_compressing_writer_sends_large_messages_smaller_and_intact() {
    let (writer, _task, mut reader, _peer) = paused_pair(1 << 20);
    let large = compressible(1);
    let frame = encoded(&large);
    let plain = writer.shared.split(frame.clone()).unwrap().bytes;
    assert!(plain >= frame.len());

    writer.compress_large_messages();
    assert!(writer.shared.split(frame.clone()).unwrap().bytes < plain / 10);
    let small = encoded(&event(2));
    assert_eq!(
        writer.shared.split(small.clone()).unwrap().bytes,
        small.len()
    );

    writer.send_reliable(&large).unwrap();
    assert_eq!(next(&mut reader).await, large);
    let lane = writer.open_render_lane(None);
    lane.enqueue_frame(encoded(&compressible(2))).unwrap();
    assert_eq!(next(&mut reader).await, compressible(2));
}

#[tokio::test]
async fn the_reader_refuses_nested_or_oversized_compressed_messages() {
    async fn read_compressed(payload: &[u8]) -> Result<ControlMessage, WireError> {
        let (local, peer) = duplex(1 << 20);
        let (_, mut peer) = split(peer);
        let mut reader = SessionReader::new(local);
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut encoder, payload).unwrap();
        let frame = encode_message(
            FrameKind::Control,
            &ControlMessage::Compressed(CompressedMessage {
                deflate: encoder.finish().unwrap(),
            }),
        )
        .unwrap();
        peer.write_all(&frame).await.unwrap();
        reader.receive().await
    }

    let plain = encoded(&event(1));
    assert_eq!(
        read_compressed(&plain[HEADER_LEN..]).await.unwrap(),
        event(1)
    );
    let nested = encode_message(
        FrameKind::Control,
        &ControlMessage::Compressed(CompressedMessage {
            deflate: Vec::new(),
        }),
    )
    .unwrap();
    assert!(matches!(
        read_compressed(&nested[HEADER_LEN..]).await,
        Err(WireError::InvalidCompression(_))
    ));
    assert!(matches!(
        read_compressed(&vec![0; MAX_FRAME_BYTES + 1]).await,
        Err(WireError::InvalidCompression(_))
    ));
}

async fn write_chunk(peer: &mut WriteHalf<DuplexStream>, stream: u64, last: bool, bytes: &[u8]) {
    let frame = encode_message(
        FrameKind::Control,
        &ControlMessage::Chunk(MessageChunk {
            stream,
            last,
            bytes: bytes.to_vec(),
        }),
    )
    .unwrap();
    peer.write_all(&frame).await.unwrap();
}

#[tokio::test]
async fn the_reader_rejects_nested_chunks_and_too_many_open_streams() {
    let (local, mut peer) = duplex(1 << 20);
    let mut reader = SessionReader::new(local);
    let inner = encode_message(
        FrameKind::Control,
        &ControlMessage::Chunk(MessageChunk {
            stream: 1,
            last: true,
            bytes: Vec::new(),
        }),
    )
    .unwrap();
    peer.write_all(
        &encode_message(
            FrameKind::Control,
            &ControlMessage::Chunk(MessageChunk {
                stream: 9,
                last: true,
                bytes: inner[HEADER_LEN..].to_vec(),
            }),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    assert!(matches!(
        reader.receive().await,
        Err(WireError::InvalidChunk(_))
    ));

    let (local, peer) = duplex(1 << 20);
    let (_, mut peer) = split(peer);
    let mut reader = SessionReader::new(local);
    for stream in 0..=MAX_OPEN_CHUNK_STREAMS as u64 {
        write_chunk(&mut peer, stream, false, b"x").await;
    }
    assert!(matches!(
        reader.receive().await,
        Err(WireError::InvalidChunk(_))
    ));
}
