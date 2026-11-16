//! 引擎状态机测试：队列不变量、播放意图、跳转、失败处理。
//!
//! 不依赖音频设备和网络：加载结果由测试直接喂给引擎，跳转由桩函数代替。

use std::sync::Mutex;

use super::recommendation_tests::fixture;
use super::*;
use crate::protocol::RecommendationSource;
use crate::testing::track;

type Seeks = Arc<Mutex<Vec<Duration>>>;

/// 记录跳转目标的桩；`fail` 为真时每次跳转都失败。
fn stub_seeks(engine: &mut Engine, fail: bool) -> Seeks {
    let seeks = Seeks::default();
    let log = seeks.clone();
    engine.seek_fn = Arc::new(move |_, position| {
        log.lock().unwrap().push(position);
        if fail { Err("decoder refused".into()) } else { Ok(()) }
    });
    seeks
}

fn tracks(mids: &[&str]) -> Vec<Track> {
    mids.iter().map(|mid| track(mid)).collect()
}

fn play(engine: &mut Engine, mids: &[&str], index: usize) {
    engine.handle(Command::Play { tracks: tracks(mids), index, recommendation: None });
}

fn prepared(engine: &Engine, position: Duration, new_stream: bool) -> Prepared {
    let mixer = engine.output.mixer.clone().unwrap();
    Prepared {
        sink: rodio::Player::connect_new(&mixer),
        stream: new_stream.then(|| StreamHandle::ready(vec![0; 1024], None)),
        hint: "mp3".into(),
        duration: Some(Duration::from_secs(180)),
        quality: Quality::High,
        position,
    }
}

/// 当前加载成功（新曲目）。
fn loaded(engine: &mut Engine) {
    let (generation, at) = (engine.generation, engine.load_at);
    let result = Ok(prepared(engine, at, true));
    engine.on_loaded(Loaded { generation, result });
}

/// 暂停后重建成功。
fn rebuilt(engine: &mut Engine) {
    let (generation, at) = (engine.generation, engine.load_at);
    let result = Ok(prepared(engine, at, false));
    engine.on_loaded(Loaded { generation, result });
}

fn failed(engine: &mut Engine, kind: ErrorKind) {
    let generation = engine.generation;
    engine.on_loaded(Loaded { generation, result: Err(LoadError::new(kind, "boom")) });
}

/// 处理下一条跳转结果（忽略真实加载任务因 mock 没有应答而回报的失败，它们的代次早已过期）。
async fn next_seek(engine: &mut Engine, internal: &mut mpsc::UnboundedReceiver<Internal>) {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(2), internal.recv()).await.unwrap().unwrap();
        if let Internal::Seeked { .. } = message {
            return engine.on_internal(message);
        }
    }
}

fn current_mid(engine: &Engine) -> Option<&str> {
    engine.index.map(|i| engine.queue[i].mid.as_str())
}

fn order(engine: &Engine) -> String {
    engine.queue.iter().map(|t| t.mid.as_str()).collect()
}

fn check_invariants(engine: &Engine) {
    assert_eq!(engine.queue.len(), engine.ids.len());
    if let Some(index) = engine.index {
        assert!(index < engine.queue.len(), "index {index} 超出队列 {}", engine.queue.len());
    }
    let mut ids = engine.ids.clone();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), engine.ids.len(), "条目 id 必须唯一");
}

fn errors(events: &mut broadcast::Receiver<Event>) -> Vec<(ErrorKind, String)> {
    let mut out = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let Event::Error { message, kind } = event {
            out.push((kind, message));
        }
    }
    out
}

#[tokio::test]
async fn empty_or_invalid_play_keeps_the_queue_usable() {
    let (mut engine, _internal, _dir, _mock) = fixture();
    let mut events = engine.events.subscribe();
    play(&mut engine, &["a", "b"], 1);
    assert_eq!(current_mid(&engine), Some("b"));

    // 越界下标：拒绝执行，队列与当前曲目不变。
    play(&mut engine, &["x"], 5);
    assert_eq!(current_mid(&engine), Some("b"));
    assert!(errors(&mut events).iter().any(|(_, m)| m.contains("超出列表范围")));
    check_invariants(&engine);

    // 空列表等同于清空：index 不能悬空。
    play(&mut engine, &[], 0);
    assert_eq!((engine.index, engine.status), (None, Status::Stopped));
    check_invariants(&engine);

    // 之后的插入不 panic，并能正常起播、继续控制。
    engine.handle(Command::PlayNext { tracks: tracks(&["n"]), tag: None });
    assert_eq!((current_mid(&engine), engine.status), (Some("n"), Status::Loading));
    engine.handle(Command::Append { tracks: tracks(&["m"]), tag: None });
    assert_eq!(current_mid(&engine), Some("n"), "追加不打断当前曲目");
    engine.handle(Command::Next);
    assert_eq!(current_mid(&engine), Some("m"));
    check_invariants(&engine);

    play(&mut engine, &[], 0);
    engine.handle(Command::Append { tracks: tracks(&["z"]), tag: None });
    assert_eq!(current_mid(&engine), Some("z"));
    for command in [
        Command::Remove { index: 9, entry: None, tag: None },
        Command::PlayIndex { index: 9, entry: None },
        Command::Move { entry: 999, to: 0, tag: None },
        Command::Append { tracks: Vec::new(), tag: None },
        Command::PlayNext { tracks: Vec::new(), tag: None },
    ] {
        engine.handle(command);
        check_invariants(&engine);
        assert_eq!(current_mid(&engine), Some("z"));
    }
}

#[tokio::test]
async fn removing_before_the_current_track_keeps_what_is_playing() {
    let (mut engine, _internal, _dir, _mock) = fixture();
    play(&mut engine, &["a", "b", "c"], 2);
    loaded(&mut engine);
    let generation = engine.generation;
    let mut states = engine.events.subscribe();

    engine.handle(Command::Remove { index: 0, entry: None, tag: None });
    assert_eq!((engine.index, current_mid(&engine)), (Some(1), Some("c")));
    assert_eq!(engine.generation, generation, "实际播放的音源没有被换掉");
    assert_eq!(engine.status, Status::Playing);

    // 对外发布的每个状态都与队列一致：下标指向的就是状态里的曲目。
    let mut queue = Vec::new();
    while let Ok(event) = states.try_recv() {
        match event {
            Event::Queue { tracks, .. } => queue = tracks,
            Event::State(state) => {
                assert_eq!(state.track.as_ref().map(|t| t.mid.as_str()), Some("c"));
                assert_eq!(queue[state.index.unwrap()].mid, "c");
                assert_eq!(state.queue_len, queue.len());
            }
            _ => {}
        }
    }
    assert_eq!(queue.len(), 2);

    // 删除当前曲目：换成顶上来的下一首；暂停时保持暂停。
    engine.handle(Command::Pause);
    engine.handle(Command::Remove { index: 0, entry: None, tag: None });
    assert_eq!((current_mid(&engine), engine.status), (Some("c"), Status::Paused));
    engine.handle(Command::Remove { index: 0, entry: None, tag: None });
    assert_eq!((engine.index, engine.status), (None, Status::Stopped));
    check_invariants(&engine);
}

#[tokio::test]
async fn stale_index_operations_do_not_hit_the_wrong_entry() {
    let (mut engine, _internal, _dir, _mock) = fixture();
    let mut events = engine.events.subscribe();
    play(&mut engine, &["a", "b", "c"], 0);
    let ids = engine.ids.clone();

    // 另一个客户端先删掉了 a；本客户端还按旧下标 1 想删 b。
    engine.handle(Command::Remove { index: 0, entry: Some(ids[0]), tag: None });
    engine.handle(Command::Remove { index: 1, entry: Some(ids[1]), tag: Some(7) });
    assert_eq!(order(&engine), "c", "删掉的是 b 而不是 c");

    // 条目已经不在队列里：拒绝并回报，不误删别的。
    engine.handle(Command::Remove { index: 0, entry: Some(ids[1]), tag: Some(8) });
    engine.handle(Command::PlayIndex { index: 0, entry: Some(ids[0]) });
    assert_eq!((order(&engine).as_str(), current_mid(&engine)), ("c", Some("c")));
    let mut done = Vec::new();
    let mut rejected = false;
    while let Ok(event) = events.try_recv() {
        match event {
            Event::Done { tag, ok, .. } => done.push((tag, ok)),
            Event::Error { message, .. } => rejected |= message.contains("已变化"),
            _ => {}
        }
    }
    assert_eq!(done, [(7, true), (8, false)]);
    assert!(rejected, "按过期条目切歌应被拒绝并提示");

    // 同一首歌重复入队：两个条目的 id 不同，可以分别操作。
    engine.handle(Command::Append { tracks: tracks(&["c", "c"]), tag: None });
    let last = *engine.ids.last().unwrap();
    engine.handle(Command::Remove { index: 0, entry: Some(last), tag: None });
    assert_eq!((order(&engine).as_str(), engine.index), ("cc", Some(0)));
    check_invariants(&engine);
}

#[tokio::test]
async fn play_next_order_append_and_move() {
    let (mut engine, _internal, _dir, _mock) = fixture();
    play(&mut engine, &["a", "b"], 0);
    // 多次「下一首播放」：后设置的先播。
    engine.handle(Command::PlayNext { tracks: tracks(&["x"]), tag: None });
    engine.handle(Command::PlayNext { tracks: tracks(&["y"]), tag: None });
    engine.handle(Command::Append { tracks: tracks(&["z"]), tag: None });
    assert_eq!(order(&engine), "ayxbz");
    assert_eq!(current_mid(&engine), Some("a"));
    assert_eq!(engine.peek_next(), Some(1));

    // 随机模式下「接下来」只承诺用户安排的条目，并且实际播放与之一致。
    engine.config.mode = PlayMode::Shuffle;
    for expected in ["y", "x"] {
        let next = engine.peek_next().unwrap();
        assert_eq!(engine.queue[next].mid, expected);
        engine.advance(true);
        assert_eq!(current_mid(&engine), Some(expected));
    }
    assert_eq!(engine.peek_next(), None, "随机挑选时不伪造顺序");

    // 移动条目：当前曲目跟着条目走。
    engine.config.mode = PlayMode::Loop;
    let current = engine.ids[engine.index.unwrap()];
    engine.handle(Command::Move { entry: current, to: 0, tag: None });
    assert_eq!((order(&engine).as_str(), engine.index), ("xaybz", Some(0)));
    engine.handle(Command::Move { entry: engine.ids[4], to: 1, tag: None });
    assert_eq!((order(&engine).as_str(), current_mid(&engine)), ("xzayb", Some("x")));
    engine.handle(Command::Move { entry: engine.ids[0], to: 99, tag: None });
    assert_eq!((order(&engine).as_str(), engine.index), ("zaybx", Some(4)));
    check_invariants(&engine);
}

#[tokio::test]
async fn shuffle_history_survives_insertions_and_removals() {
    let (mut engine, _internal, _dir, _mock) = fixture();
    play(&mut engine, &["a", "b", "c"], 0);
    engine.config.mode = PlayMode::Shuffle;
    engine.handle(Command::PlayIndex { index: 1, entry: None });
    engine.handle(Command::PlayIndex { index: 2, entry: None });
    // 历史是 a、b；在它们前后插入、删除，下标全部错位。
    engine.handle(Command::PlayNext { tracks: tracks(&["x", "y"]), tag: None });
    let a = engine.position_of(engine.ids[0]).unwrap();
    engine.handle(Command::Move { entry: engine.ids[a], to: 4, tag: None });
    assert_eq!(order(&engine), "bcxya");
    engine.handle(Command::Prev);
    assert_eq!(current_mid(&engine), Some("b"));
    engine.handle(Command::Prev);
    assert_eq!(current_mid(&engine), Some("a"));

    // 历史里的条目被删掉后直接跳过。
    engine.handle(Command::PlayIndex { index: 1, entry: None });
    engine.handle(Command::PlayIndex { index: 2, entry: None });
    engine.handle(Command::Remove { index: 1, entry: None, tag: None });
    assert_eq!(current_mid(&engine), Some("x"));
    engine.handle(Command::Prev);
    assert_eq!(current_mid(&engine), Some("a"));
    check_invariants(&engine);
}

#[tokio::test]
async fn pause_survives_a_failed_load_and_only_resume_plays() {
    let (mut engine, _internal, _dir, _mock) = fixture();
    play(&mut engine, &["a", "b"], 0);
    engine.handle(Command::Pause);
    assert_eq!(engine.status, Status::Paused);
    let mut events = engine.events.subscribe();

    // A 加载失败：换到 B，但仍然是暂停。
    failed(&mut engine, ErrorKind::Rights);
    assert_eq!((current_mid(&engine), engine.status), (Some("b"), Status::Paused));
    assert!(engine.loading);

    // B 加载成功：依然不出声。
    loaded(&mut engine);
    assert_eq!(engine.status, Status::Paused);
    assert!(engine.current.as_ref().unwrap().sink.is_none());
    // 期间发布的状态里没有出现过「正在播放 / 加载」的假象。
    while let Ok(event) = events.try_recv() {
        if let Event::State(state) = event {
            assert_eq!(state.status, Status::Paused);
        }
    }

    // 只有 Resume 才开始播放。
    engine.handle(Command::Resume);
    assert_eq!(engine.status, Status::Loading);
    rebuilt(&mut engine);
    assert_eq!(engine.status, Status::Playing);
    assert!(engine.current.as_ref().unwrap().sink.is_some());

    // 切换音质也不会让暂停的曲目出声。
    engine.handle(Command::Pause);
    engine.handle(Command::SetQuality { quality: Quality::Standard });
    assert_eq!(engine.status, Status::Paused);
    loaded(&mut engine);
    assert_eq!(engine.status, Status::Paused);
}

#[tokio::test]
async fn paused_radio_does_not_fetch_or_play_after_a_failure() {
    let (mut engine, _internal, _dir, mock) = fixture();
    let source = RecommendationSource::Guess;
    engine.handle(Command::Play { tracks: tracks(&["first", "last"]), index: 1, recommendation: Some(source) });
    mock.clear_requests();
    engine.handle(Command::Pause);
    failed(&mut engine, ErrorKind::Rights);
    // 队列里没有下一首：等待推荐，但暂停时不去请求。
    assert_eq!(engine.status, Status::Paused);
    assert!(engine.radio.as_ref().unwrap().waiting);
    engine.on_tick(1);
    assert!(engine.radio.as_ref().unwrap().pending.is_none());
}

#[tokio::test]
async fn results_of_superseded_loads_are_ignored() {
    let (mut engine, _internal, _dir, _mock) = fixture();
    play(&mut engine, &["a", "b"], 0);
    let old = engine.generation;
    let old_task = engine.load_task.clone().unwrap();

    // Next 之后，A 的加载任务被中止，它的结果（成功或失败）都不能影响 B。
    engine.handle(Command::Next);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !old_task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("旧加载任务应在期限内结束");
    let result = Ok(prepared(&engine, Duration::ZERO, true));
    engine.on_loaded(Loaded { generation: old, result });
    assert_eq!((current_mid(&engine), engine.status), (Some("b"), Status::Loading));
    assert!(engine.current.is_none());
    engine.on_loaded(Loaded { generation: old, result: Err(LoadError::new(ErrorKind::Network, "late")) });
    assert_eq!((current_mid(&engine), engine.fail_streak), (Some("b"), 0));

    // Stop 之后同理：不会被旧任务重新拉起，也不会被旧失败推进到下一首。
    let old = engine.generation;
    engine.handle(Command::Stop);
    let result = Ok(prepared(&engine, Duration::ZERO, true));
    engine.on_loaded(Loaded { generation: old, result });
    engine.on_loaded(Loaded { generation: old, result: Err(LoadError::new(ErrorKind::Rights, "late")) });
    assert_eq!((current_mid(&engine), engine.status), (Some("b"), Status::Stopped));
    assert!(engine.current.is_none());
    assert_eq!(engine.activity(), Activity::Idle);
}

#[tokio::test]
async fn seek_while_playing_keeps_only_the_latest_target() {
    let (mut engine, mut internal, _dir, _mock) = fixture();
    let seeks = stub_seeks(&mut engine, false);
    play(&mut engine, &["a"], 0);
    loaded(&mut engine);

    let secs = Duration::from_secs;
    engine.handle(Command::Seek { position_ms: 10_000 });
    engine.handle(Command::Seek { position_ms: 20_000 });
    engine.handle(Command::Seek { position_ms: 30_000 });
    assert_eq!(engine.position(), secs(30), "对外报告的是最新目标");
    next_seek(&mut engine, &mut internal).await;
    next_seek(&mut engine, &mut internal).await;
    assert_eq!(*seeks.lock().unwrap(), [secs(10), secs(30)], "中间的目标被跳过");
    assert!(engine.seeking.is_none() && engine.target.is_none());
    assert_eq!(engine.status, Status::Playing);

    // 超出时长的位置被限制在合法范围内；相对跳转不会小于 0。
    engine.handle(Command::Seek { position_ms: 999_000 });
    next_seek(&mut engine, &mut internal).await;
    assert_eq!(seeks.lock().unwrap().last().copied(), Some(Duration::from_millis(179_500)));
    engine.handle(Command::SeekBy { offset_ms: -999_000 });
    next_seek(&mut engine, &mut internal).await;
    assert_eq!(seeks.lock().unwrap().last().copied(), Some(Duration::ZERO));
}

#[tokio::test]
async fn seek_failure_is_reported() {
    let (mut engine, mut internal, _dir, _mock) = fixture();
    stub_seeks(&mut engine, true);
    play(&mut engine, &["a"], 0);
    loaded(&mut engine);
    let mut events = engine.events.subscribe();
    engine.handle(Command::Seek { position_ms: 10_000 });
    next_seek(&mut engine, &mut internal).await;
    let errors = errors(&mut events);
    assert!(errors.iter().any(|(kind, m)| *kind == ErrorKind::Local && m.contains("跳转失败")), "{errors:?}");
    assert!(engine.seeking.is_none());
    assert_eq!(engine.status, Status::Playing);
}

#[tokio::test]
async fn seek_while_paused_moves_the_position_without_resuming() {
    let (mut engine, mut internal, _dir, _mock) = fixture();
    let seeks = stub_seeks(&mut engine, false);
    play(&mut engine, &["a"], 0);
    loaded(&mut engine);
    engine.handle(Command::Pause);
    engine.handle(Command::Seek { position_ms: 42_000 });
    assert_eq!((engine.status, engine.position()), (Status::Paused, Duration::from_secs(42)));
    assert!(seeks.lock().unwrap().is_empty(), "暂停时没有音源可跳");

    // 恢复时从新位置重建。
    engine.handle(Command::Resume);
    assert_eq!(engine.load_at, Duration::from_secs(42));

    // 重建在途时又跳转：重建完成后补跳到最新目标，然后才出声。
    engine.handle(Command::Seek { position_ms: 50_000 });
    engine.handle(Command::Seek { position_ms: 60_000 });
    rebuilt(&mut engine);
    assert_eq!(engine.status, Status::Loading, "跳转完成前不出声");
    next_seek(&mut engine, &mut internal).await;
    assert_eq!(*seeks.lock().unwrap(), [Duration::from_secs(60)]);
    assert_eq!(engine.status, Status::Playing);

    // 跳转途中暂停：目标折算成暂停位置，迟到的跳转结果不会让它恢复播放。
    engine.handle(Command::Seek { position_ms: 70_000 });
    engine.handle(Command::Pause);
    assert_eq!((engine.status, engine.position()), (Status::Paused, Duration::from_secs(70)));
    next_seek(&mut engine, &mut internal).await;
    assert_eq!((engine.status, engine.position()), (Status::Paused, Duration::from_secs(70)));
}

#[tokio::test]
async fn seek_during_the_initial_load_and_quality_reload() {
    let (mut engine, mut internal, _dir, _mock) = fixture();
    let seeks = stub_seeks(&mut engine, false);
    play(&mut engine, &["a"], 0);

    // 初次加载期间先暂停再跳转：目标不丢，加载完成后停在目标位置。
    engine.handle(Command::Pause);
    engine.handle(Command::Seek { position_ms: 15_000 });
    assert_eq!(engine.position(), Duration::from_secs(15));
    loaded(&mut engine);
    assert_eq!((engine.status, engine.position()), (Status::Paused, Duration::from_secs(15)));
    engine.handle(Command::Resume);
    assert_eq!(engine.load_at, Duration::from_secs(15));
    rebuilt(&mut engine);
    assert_eq!(engine.status, Status::Playing);

    // 未暂停的初次加载：加载完成后先跳到最新目标再出声。
    engine.handle(Command::Stop);
    engine.handle(Command::Resume);
    engine.handle(Command::Seek { position_ms: 5_000 });
    engine.handle(Command::Seek { position_ms: 25_000 });
    loaded(&mut engine);
    assert_eq!(engine.status, Status::Loading);
    next_seek(&mut engine, &mut internal).await;
    assert_eq!(seeks.lock().unwrap().last().copied(), Some(Duration::from_secs(25)));
    assert_eq!(engine.status, Status::Playing);

    // 切换音质：从当前位置重新加载；加载中再切一次同样以当前位置为准。
    engine.handle(Command::Pause);
    engine.handle(Command::Seek { position_ms: 90_000 });
    engine.handle(Command::Resume);
    engine.handle(Command::SetQuality { quality: Quality::Lossless });
    assert_eq!((engine.load_at, engine.status), (Duration::from_secs(90), Status::Loading));
    engine.handle(Command::SetQuality { quality: Quality::Standard });
    assert_eq!(engine.load_at, Duration::from_secs(90));
    loaded(&mut engine);
    assert_eq!(engine.status, Status::Playing);
    assert_eq!(engine.current.as_ref().unwrap().paused_at, Duration::from_secs(90));
}

#[tokio::test]
async fn consecutive_failures_stop_instead_of_skipping_forever() {
    // 普通失败：最多连续跳 10 首。
    let (mut engine, _internal, _dir, _mock) = fixture();
    let mut events = engine.events.subscribe();
    let mids: Vec<String> = (0..50).map(|i| format!("s{i}")).collect();
    play(&mut engine, &mids.iter().map(String::as_str).collect::<Vec<_>>(), 0);
    let mut skipped = 0;
    while engine.status != Status::Stopped {
        failed(&mut engine, ErrorKind::Rights);
        skipped += 1;
        assert!(skipped <= 50, "不应无限跳歌");
    }
    assert_eq!(skipped, MAX_FAIL_STREAK);
    assert!(errors(&mut events).iter().any(|(_, m)| m.contains("连续 10 首无法播放")));
    check_invariants(&engine);

    // 网络失败更早停止；登录失效立即停止。
    for (kind, expected) in [(ErrorKind::Network, MAX_NETWORK_FAIL_STREAK), (ErrorKind::Login, 1)] {
        engine.handle(Command::Resume);
        let mut failures = 0;
        while engine.status != Status::Stopped {
            failed(&mut engine, kind);
            failures += 1;
        }
        assert_eq!(failures, expected, "{kind:?}");
    }

    // 成功一次后重新计数。
    engine.handle(Command::Resume);
    failed(&mut engine, ErrorKind::Rights);
    loaded(&mut engine);
    assert_eq!(engine.fail_streak, 0);
}

#[tokio::test]
async fn interrupted_download_retries_once_from_the_break_point() {
    let (mut engine, _internal, _dir, _mock) = fixture();
    let mut events = engine.events.subscribe();
    play(&mut engine, &["a", "b"], 0);
    let broken = |engine: &Engine, at: Duration| {
        let mut prepared = prepared(engine, at, false);
        prepared.stream = Some(StreamHandle::ready(vec![0; 1024], Some("下载中断: reset".into())));
        Loaded { generation: engine.generation, result: Ok(prepared) }
    };
    let load = broken(&engine, Duration::ZERO);
    engine.on_loaded(load);
    assert_eq!(engine.status, Status::Playing);

    // 音源读到断点结束：不是自然播完，从断点重新加载同一首。
    engine.last_pos = Duration::from_secs(33);
    engine.on_tick(1);
    assert_eq!((current_mid(&engine), engine.status), (Some("a"), Status::Loading));
    assert_eq!(engine.load_at, Duration::from_secs(33));
    let reported = errors(&mut events);
    assert!(reported.iter().any(|(kind, m)| *kind == ErrorKind::Network && m.contains("断点")), "{reported:?}");

    // 重试后再次中断：不再重试，按失败处理换下一首。
    let load = broken(&engine, Duration::from_secs(33));
    engine.on_loaded(load);
    engine.on_tick(2);
    assert_eq!(current_mid(&engine), Some("b"));

    // 正常下载完的曲目播完后照常切歌。
    loaded(&mut engine);
    engine.on_tick(3);
    assert_eq!(current_mid(&engine), Some("a"));
    assert_eq!(engine.load_at, Duration::ZERO);
}

#[tokio::test]
async fn state_reports_intent_and_activity_separately() {
    let (mut engine, _internal, _dir, _mock) = fixture();
    let state = |engine: &Engine| {
        engine.publish();
        engine.state_tx.borrow().clone()
    };
    assert_eq!(state(&engine).protocol, PROTOCOL_VERSION);
    play(&mut engine, &["a", "b"], 0);
    let s = state(&engine);
    assert_eq!((s.status, s.activity, s.next), (Status::Loading, Activity::Loading, Some(1)));
    assert_eq!(s.entry, Some(engine.ids[0]));
    engine.handle(Command::Pause);
    let s = state(&engine);
    assert_eq!((s.status, s.activity), (Status::Paused, Activity::Loading), "暂停意图与仍在加载可以同时表达");
    loaded(&mut engine);
    assert_eq!(state(&engine).activity, Activity::Idle);
    engine.handle(Command::Resume);
    rebuilt(&mut engine);
    let s = state(&engine);
    assert_eq!((s.status, s.activity), (Status::Playing, Activity::Output));
    engine.handle(Command::Stop);
    let s = state(&engine);
    assert_eq!((s.status, s.activity), (Status::Stopped, Activity::Idle));
}

#[tokio::test]
async fn credential_reload_reports_its_result() {
    let (mut engine, _internal, _dir, _mock) = fixture();
    let mut events = engine.events.subscribe();
    crate::save_credential(&engine.paths, &qqmusic_api::Credential::new(42, "key")).unwrap();
    engine.handle(Command::ReloadCredential { tag: Some(1) });
    assert_eq!(engine.client.credential().musicid, 42);

    // 凭证文件损坏：回报失败，并保留原来的登录状态。
    std::fs::write(engine.paths.credential(), "{ broken").unwrap();
    engine.handle(Command::ReloadCredential { tag: Some(2) });
    assert_eq!(engine.client.credential().musicid, 42);

    // 凭证被删除（退出登录）：成功，变为未登录。
    crate::remove_credential(&engine.paths).unwrap();
    engine.handle(Command::ReloadCredential { tag: Some(3) });
    assert!(!engine.client.credential().is_valid());

    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            Event::Done { tag, ok, .. } => seen.push(format!("done {tag} {ok}")),
            Event::CredentialChanged { logged_in, musicid } => seen.push(format!("changed {logged_in} {musicid}")),
            _ => {}
        }
    }
    assert_eq!(seen, ["done 1 true", "changed true 42", "done 2 false", "done 3 true", "changed false 0"]);
}
