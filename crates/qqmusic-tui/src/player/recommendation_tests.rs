use super::*;
use crate::protocol::RecommendationSource;
use crate::testing::{client, reply, track};
use qqmusic_api::transport::mock::MockTransport;
use serde_json::json;

fn fixture() -> (Engine, mpsc::UnboundedReceiver<Loaded>, tempfile::TempDir, MockTransport) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::with_root(dir.path()).unwrap();
    let mock = MockTransport::new();
    let client = client(&mock);
    let (events, _) = broadcast::channel(64);
    let (state_tx, _) = watch::channel(PlayerState::default());
    let (queue_tx, _) = watch::channel(Arc::new(Vec::new()));
    let (load_tx, loads) = mpsc::unbounded_channel();
    let (mixer, _) = rodio::mixer::mixer(2.try_into().unwrap(), 44100.try_into().unwrap());
    let engine = Engine {
        cdn: Arc::new(cdn::CdnSelector::new(&client)),
        client,
        paths,
        config: Config { fade_ms: 0, ..Default::default() },
        output: Output { mixer: Some(mixer), close: None },
        fade_token: Arc::default(),
        idle_ticks: 0,
        queue: Vec::new(),
        index: None,
        history: Vec::new(),
        radio: None,
        status: Status::Stopped,
        current: None,
        generation: 0,
        resume_at: None,
        playing_quality: None,
        duration_ms: 0,
        queue_version: 0,
        fail_streak: 0,
        load_tx,
        events,
        state_tx,
        queue_tx,
    };
    (engine, loads, dir, mock)
}

fn seed(engine: &mut Engine) {
    engine.queue = vec![track("old")];
    engine.index = Some(0);
    engine.status = Status::Playing;
    let mut radio = radio::Radio::new(RecommendationSource::Radar { next_page: 3 }, &engine.queue);
    radio.played.insert("old".into());
    engine.radio = Some(radio);
}

#[tokio::test]
async fn next_batch_starts_only_after_the_last_track_ends_and_requests_once() {
    let (mut engine, _loads, _dir, mock) = fixture();
    seed(&mut engine);
    engine.queue.extend((1..20).map(|n| track(&format!("initial-{n}"))));
    mock.push_response(reply(json!({"VecSongs": [{"Track": {"id": 1, "mid": "new"}}], "HasMore": false})));
    for index in [0, 17, 19] {
        engine.index = Some(index);
        engine.on_tick(1);
        assert!(engine.radio.as_ref().unwrap().pending.is_none());
        assert_eq!(mock.request_count(), 0);
    }
    // 加载或播放最后一首时都不请求下一轮。
    engine.status = Status::Loading;
    engine.on_tick(1);
    assert!(engine.radio.as_ref().unwrap().pending.is_none());
    engine.status = Status::Playing;
    engine.on_track_end();
    assert!(engine.radio.as_ref().unwrap().waiting);
    let task = engine.radio.as_ref().unwrap().pending.as_ref().unwrap().id();
    for _ in 0..10 {
        engine.on_tick(1);
        assert_eq!(engine.radio.as_ref().unwrap().pending.as_ref().unwrap().id(), task);
    }
    radio::next_batch(&mut engine.radio).await.unwrap();
    assert_eq!(mock.request_count(), 1);
}

#[tokio::test]
async fn background_engine_fetches_the_next_batch_without_a_tui_connection() {
    let (mut engine, loads, _dir, mock) = fixture();
    seed(&mut engine);
    mock.push_response(reply(json!({"VecSongs": [{"Track": {"id": 1, "mid": "new"}}], "HasMore": false})));
    let mut queue = engine.queue_tx.subscribe();
    engine.on_track_end();
    let (commands, rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(engine.run(rx, loads));
    tokio::time::timeout(Duration::from_secs(2), queue.changed()).await.unwrap().unwrap();
    assert_eq!(queue.borrow().iter().map(|t| t.mid.as_str()).collect::<Vec<_>>(), ["old", "new"]);
    assert_eq!(mock.requests().first().unwrap().json_body().unwrap()["req_0"]["param"]["Page"], 3);
    commands.send(Command::Shutdown).unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn end_waits_for_fresh_tracks_instead_of_looping_and_deduplicates() {
    let (mut engine, _loads, _dir, mock) = fixture();
    seed(&mut engine);
    mock.push_response(reply(json!({"VecSongs": [{"Track": {"id": 1, "mid": "new"}}], "HasMore": false})));
    engine.on_track_end();
    assert_eq!(engine.index, Some(0));
    assert_eq!(engine.status, Status::Loading);
    assert!(engine.radio.as_ref().unwrap().waiting);
    let mut batch = radio::next_batch(&mut engine.radio).await.unwrap();
    batch.tracks.extend([track("old"), track("new"), track("")]);
    engine.on_recommendation(Ok(batch));
    assert_eq!(engine.queue.len(), 2);
    assert_eq!(engine.index, Some(1));
    assert_eq!(engine.radio.as_ref().unwrap().source, RecommendationSource::Radar { next_page: 1 });
    assert!(!engine.radio.as_ref().unwrap().waiting);
}

#[tokio::test]
async fn completion_while_paused_does_not_restart_playback() {
    let (mut engine, _loads, _dir, mock) = fixture();
    seed(&mut engine);
    mock.push_response(reply(json!({"VecSongs": [{"Track": {"id": 1, "mid": "new"}}], "HasMore": false})));
    engine.on_track_end();
    engine.pause();
    let batch = radio::next_batch(&mut engine.radio).await.unwrap();
    engine.on_recommendation(Ok(batch));
    assert_eq!(engine.status, Status::Paused);
    assert_eq!(engine.index, Some(0));
    engine.resume();
    assert_eq!(engine.index, Some(1));
    assert_eq!(engine.status, Status::Loading);
}

#[tokio::test]
async fn stop_clear_and_replacing_queue_cancel_old_requests() {
    for command in [Command::Stop, Command::Clear, Command::Play { tracks: Vec::new(), index: 0, recommendation: None }]
    {
        let (mut engine, _loads, _dir, _mock) = fixture();
        seed(&mut engine);
        let pending = tokio::spawn(std::future::pending::<Result<Batch, String>>());
        let abort = pending.abort_handle();
        engine.radio.as_mut().unwrap().pending = Some(pending);
        engine.handle(command);
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
        assert_eq!(engine.status, Status::Stopped);
        assert!(engine.radio.as_ref().is_none_or(|r| r.pending.is_none() && !r.waiting));
    }
}

#[tokio::test(start_paused = true)]
async fn empty_batches_back_off_and_stop_after_three_attempts() {
    let (mut engine, _loads, _dir, mock) = fixture();
    seed(&mut engine);
    for _ in 0..3 {
        mock.push_response(reply(json!({"VecSongs": [], "HasMore": false})));
    }
    engine.on_track_end();
    for attempt in 1..=3 {
        let batch = radio::next_batch(&mut engine.radio).await.unwrap();
        engine.on_recommendation(Ok(batch));
        assert_eq!(engine.radio.as_ref().unwrap().failures, attempt);
        engine.fetch_recommendations();
        assert!(engine.radio.as_ref().unwrap().pending.is_none());
        if attempt < 3 {
            tokio::time::advance(Duration::from_secs(5 << (attempt - 1))).await;
            engine.fetch_recommendations();
        }
    }
    assert_eq!(engine.status, Status::Stopped);
    assert_eq!(mock.request_count(), 3);
    tokio::time::advance(Duration::from_secs(600)).await;
    engine.on_tick(100);
    assert!(engine.radio.as_ref().unwrap().pending.is_none());
}

#[tokio::test]
async fn request_errors_preserve_current_queue_and_use_the_same_retry_limit() {
    let (mut engine, _loads, _dir, _mock) = fixture();
    seed(&mut engine);
    engine.status = Status::Loading;
    engine.radio.as_mut().unwrap().waiting = true;
    for attempt in 1..=3 {
        engine.on_recommendation(Err("network unavailable".into()));
        assert_eq!(engine.status, if attempt < 3 { Status::Loading } else { Status::Stopped });
        assert_eq!(engine.queue, [track("old")]);
    }
}

#[tokio::test]
async fn single_repeat_and_ordinary_playlist_modes_keep_their_behavior() {
    let (mut engine, _loads, _dir, _mock) = fixture();
    seed(&mut engine);
    engine.config.mode = PlayMode::Single;
    assert_eq!(engine.next_index(true), Some(0));
    engine.fetch_recommendations();
    assert!(engine.radio.as_ref().unwrap().pending.is_none());
    engine.advance(false);
    assert!(engine.radio.as_ref().unwrap().waiting);
    engine.radio = None;
    engine.config.mode = PlayMode::Loop;
    assert_eq!(engine.next_index(true), Some(0));
    engine.config.mode = PlayMode::Order;
    assert_eq!(engine.next_index(true), None);
}

#[tokio::test]
async fn shuffled_radio_consumes_unplayed_songs_before_waiting_for_more() {
    let (mut engine, _loads, _dir, _mock) = fixture();
    seed(&mut engine);
    engine.queue.extend([track("second"), track("third")]);
    engine.config.mode = PlayMode::Shuffle;
    let first = engine.next_index(true).unwrap();
    assert_ne!(first, 0);
    engine.radio.as_mut().unwrap().played.insert(engine.queue[first].mid.clone());
    let second = engine.next_index(true).unwrap();
    assert_ne!(second, first);
    assert_ne!(second, 0);
    engine.radio.as_mut().unwrap().played.insert(engine.queue[second].mid.clone());
    assert_eq!(engine.next_index(true), None);
}

#[tokio::test]
async fn manually_appended_song_resumes_a_waiting_radio() {
    let (mut engine, _loads, _dir, _mock) = fixture();
    seed(&mut engine);
    engine.on_track_end();
    engine.handle(Command::Append { tracks: vec![track("manual")] });
    assert_eq!(engine.index, Some(1));
    assert!(!engine.radio.as_ref().unwrap().waiting);
    assert!(engine.radio.as_ref().unwrap().seen.contains("manual"));
}
