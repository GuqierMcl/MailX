use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::*;
use crate::imap::{TestPeer, scripted_session};

const IDLE_CAPS: &str = "IMAP4rev1 IDLE";

type Script = JoinHandle<()>;

/// Hands the watcher one prepared session per connection attempt; once they
/// run out, further attempts hang so the test can see the watcher waiting.
struct Harness {
    sessions: Arc<Mutex<VecDeque<Session>>>,
    health: Arc<watch::Sender<IdleHealth>>,
    control: Arc<IdleControl>,
    wakes: Arc<AtomicUsize>,
    watcher: JoinHandle<()>,
}

impl Harness {
    async fn start(sessions: Vec<Session>) -> Self {
        let sessions = Arc::new(Mutex::new(VecDeque::from(sessions)));
        let health = Arc::new(watch::channel(IdleHealth::Starting).0);
        let control = Arc::new(IdleControl::default());
        let sync_wake = Arc::new(Notify::new());
        let wakes = Arc::new(AtomicUsize::new(0));
        {
            let (sync_wake, wakes) = (sync_wake.clone(), wakes.clone());
            tokio::spawn(async move {
                loop {
                    sync_wake.notified().await;
                    wakes.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
        let context = WatchContext {
            health: health.clone(),
            sync_wake,
            control: control.clone(),
            timing: Timing::default(),
        };
        let queue = sessions.clone();
        let watcher = tokio::spawn(watch(
            move || {
                let next = queue.lock().unwrap().pop_front();
                async move {
                    match next {
                        Some(session) => Ok(session),
                        None => std::future::pending().await,
                    }
                }
            },
            context,
        ));
        Self {
            sessions,
            health,
            control,
            wakes,
            watcher,
        }
    }

    fn health(&self) -> IdleHealth {
        *self.health.borrow()
    }

    fn wakes(&self) -> usize {
        self.wakes.load(Ordering::SeqCst)
    }

    fn unused_sessions(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
}

async fn session<F, Fut>(capabilities: &str, script: F) -> (Session, Script)
where
    F: FnOnce(TestPeer) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    scripted_session(capabilities, script).await
}

/// Lets paused time run `duration` forward, with every task getting its turn.
async fn pass(duration: Duration) {
    tokio::time::sleep(duration).await;
}

#[tokio::test(start_paused = true)]
async fn a_server_without_idle_is_unsupported_and_never_sees_idle() {
    // The script would panic on any command other than LOGOUT.
    let (client, server) = session("IMAP4rev1", |mut peer| async move {
        peer.logout().await;
    })
    .await;
    let mut harness = Harness::start(vec![client]).await;
    (&mut harness.watcher).await.unwrap();
    server.await.unwrap();
    assert_eq!(harness.health.borrow().clone(), IdleHealth::Unsupported);
    assert_eq!(harness.wakes(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_burst_of_pushes_wakes_sync_once_after_the_debounce() {
    let (client, server) = session(IDLE_CAPS, |mut peer| async move {
        peer.examine().await;
        let tag = peer.enter_idle().await;
        peer.send("* 2 EXISTS\r\n").await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        peer.send("* 3 EXISTS\r\n").await;
        peer.leave_idle(&tag).await;
        peer.logout().await;
    })
    .await;
    let mut harness = Harness::start(vec![client]).await;
    pass(Duration::from_millis(900)).await;
    assert_eq!(harness.health(), IdleHealth::Live);
    // Only the catch-up wake from connecting so far: the window is still open.
    assert_eq!(harness.wakes(), 1);
    pass(Duration::from_secs(2)).await;
    assert_eq!(harness.wakes(), 2, "two pushes in the window, one wake");
    pass(Duration::from_secs(30)).await;
    assert_eq!(harness.wakes(), 2);

    harness.control.stop.notify_one();
    (&mut harness.watcher).await.unwrap();
    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn reconnecting_after_a_disconnect_wakes_sync_once_more() {
    let (first, first_server) = session(IDLE_CAPS, |mut peer| async move {
        peer.examine().await;
        peer.enter_idle().await;
        peer.send("* BYE going away\r\n").await;
    })
    .await;
    let (second, second_server) = session(IDLE_CAPS, |mut peer| async move {
        peer.examine().await;
        let tag = peer.enter_idle().await;
        peer.leave_idle(&tag).await;
        peer.logout().await;
    })
    .await;
    let mut harness = Harness::start(vec![first, second]).await;
    pass(Duration::from_millis(100)).await;
    assert_eq!(harness.wakes(), 1);
    assert_eq!(harness.health(), IdleHealth::Reconnecting);
    pass(Duration::from_secs(300)).await;
    assert_eq!(harness.health(), IdleHealth::Live);
    assert_eq!(harness.wakes(), 2, "one wake per successful connection");

    harness.control.stop.notify_one();
    (&mut harness.watcher).await.unwrap();
    first_server.await.unwrap();
    second_server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_provider_refusal_disables_idle_without_a_reconnect_loop() {
    let (refused, server) = session(IDLE_CAPS, |mut peer| async move {
        peer.examine().await;
        let tag = peer.expect("IDLE").await;
        peer.send(&format!(
            "{tag} NO Unsafe Login. Please contact kefu@188.com for help\r\n"
        ))
        .await;
    })
    .await;
    // A spare session proves nothing tries again.
    let (spare, spare_server) = session(IDLE_CAPS, |_peer| async {}).await;
    let mut harness = Harness::start(vec![refused, spare]).await;
    (&mut harness.watcher).await.unwrap();
    server.await.unwrap();
    assert_eq!(harness.health(), IdleHealth::Disabled);
    assert_eq!(harness.unused_sessions(), 1);
    assert_eq!(harness.wakes(), 0);
    spare_server.abort();
}

#[tokio::test(start_paused = true)]
async fn shutdown_sends_done_then_logout() {
    let (client, server) = session(IDLE_CAPS, |mut peer| async move {
        peer.examine().await;
        let tag = peer.enter_idle().await;
        peer.leave_idle(&tag).await;
        peer.logout().await;
    })
    .await;
    let mut harness = Harness::start(vec![client]).await;
    pass(Duration::from_secs(1)).await;
    assert_eq!(harness.health(), IdleHealth::Live);
    harness.control.stop.notify_one();
    (&mut harness.watcher).await.unwrap();
    // The script only finishes if DONE and LOGOUT arrived in order.
    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn idle_is_re_entered_every_ten_minutes_without_waking_sync() {
    let (client, server) = session(IDLE_CAPS, |mut peer| async move {
        peer.examine().await;
        let tag = peer.enter_idle().await;
        peer.leave_idle(&tag).await;
        let tag = peer.enter_idle().await;
        peer.leave_idle(&tag).await;
        peer.logout().await;
    })
    .await;
    let mut harness = Harness::start(vec![client]).await;
    pass(Duration::from_secs(9 * 60)).await;
    assert!(!harness.watcher.is_finished());
    pass(Duration::from_secs(2 * 60)).await;
    assert_eq!(harness.health(), IdleHealth::Live);
    assert_eq!(harness.wakes(), 1);

    harness.control.stop.notify_one();
    (&mut harness.watcher).await.unwrap();
    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn too_many_pushes_drop_the_connection_and_back_off() {
    let (flooded, flooded_server) = session(IDLE_CAPS, |mut peer| async move {
        peer.examine().await;
        let tag = peer.enter_idle().await;
        let burst: String = (1..=70).map(|n| format!("* {n} EXISTS\r\n")).collect();
        peer.send(&burst).await;
        peer.leave_idle(&tag).await;
        peer.logout().await;
    })
    .await;
    let (second, second_server) = session(IDLE_CAPS, |mut peer| async move {
        peer.examine().await;
        let tag = peer.enter_idle().await;
        peer.leave_idle(&tag).await;
        peer.logout().await;
    })
    .await;
    let mut harness = Harness::start(vec![flooded, second]).await;
    pass(Duration::from_millis(500)).await;
    assert_eq!(harness.health(), IdleHealth::Reconnecting);
    assert_eq!(harness.unused_sessions(), 1, "backing off, not yet retried");
    pass(Duration::from_secs(300)).await;
    assert_eq!(harness.health(), IdleHealth::Live);

    harness.control.stop.notify_one();
    (&mut harness.watcher).await.unwrap();
    flooded_server.await.unwrap();
    second_server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_sleep_signal_reconnects_immediately_and_wakes_sync() {
    let (first, first_server) = session(IDLE_CAPS, |mut peer| async move {
        peer.examine().await;
        let tag = peer.enter_idle().await;
        peer.leave_idle(&tag).await;
        peer.logout().await;
    })
    .await;
    let (second, second_server) = session(IDLE_CAPS, |mut peer| async move {
        peer.examine().await;
        let tag = peer.enter_idle().await;
        peer.leave_idle(&tag).await;
        peer.logout().await;
    })
    .await;
    let mut harness = Harness::start(vec![first, second]).await;
    pass(Duration::from_secs(1)).await;
    assert_eq!(harness.wakes(), 1);
    harness.control.reconnect.notify_one();
    pass(Duration::from_millis(100)).await;
    assert_eq!(harness.health(), IdleHealth::Live);
    assert_eq!(harness.wakes(), 2);
    assert_eq!(harness.unused_sessions(), 0);

    harness.control.stop.notify_one();
    (&mut harness.watcher).await.unwrap();
    first_server.await.unwrap();
    second_server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn credential_failures_disable_idle() {
    let (spare, spare_server) = session(IDLE_CAPS, |_peer| async {}).await;
    let queue = Arc::new(Mutex::new(VecDeque::from([spare])));
    let attempts = Arc::new(AtomicU32::new(0));
    let health = Arc::new(watch::channel(IdleHealth::Starting).0);
    let context = WatchContext {
        health: health.clone(),
        sync_wake: Arc::new(Notify::new()),
        control: Arc::new(IdleControl::default()),
        timing: Timing::default(),
    };
    let counter = attempts.clone();
    watch(
        move || {
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = &queue;
            async { Err(ConnectFailure::Imap(ImapError::Authentication("no".into()))) }
        },
        context,
    )
    .await;
    assert_eq!(*health.borrow(), IdleHealth::Disabled);
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    spare_server.abort();
}

#[test]
fn sleep_detector_ignores_normal_drift_and_clock_steps_back() {
    let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    let mono = Instant::now();
    let mut detector = SleepDetector::new(wall, mono);
    let secs = Duration::from_secs;
    assert!(!detector.check(wall + secs(30), mono + secs(30)));
    // Timer lateness and small NTP slews stay under the threshold.
    assert!(!detector.check(wall + secs(120), mono + secs(65)));
    // The wall clock being set back is not a sleep.
    assert!(!detector.check(wall + secs(10), mono + secs(95)));
}

#[test]
fn sleep_detector_flags_a_wall_clock_jump_once() {
    let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    let mono = Instant::now();
    let mut detector = SleepDetector::new(wall, mono);
    let secs = Duration::from_secs;
    // 30 s of monotonic time, but 8 hours of wall time.
    assert!(detector.check(wall + secs(8 * 3600), mono + secs(30)));
    assert!(!detector.check(wall + secs(8 * 3600 + 30), mono + secs(60)));
}

#[tokio::test(start_paused = true)]
async fn the_ticker_reports_a_suspend_but_not_a_quiet_minute() {
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    let start = Instant::now();
    let slept = Arc::new(AtomicU32::new(0));
    let reports = Arc::new(AtomicU32::new(0));
    let ticker = {
        let (slept, reports) = (slept.clone(), reports.clone());
        tokio::spawn(sleep_ticker(
            move || {
                base + start.elapsed()
                    + Duration::from_secs(u64::from(slept.load(Ordering::SeqCst)))
            },
            Instant::now,
            move || {
                reports.fetch_add(1, Ordering::SeqCst);
            },
        ))
    };
    pass(Duration::from_secs(5 * 60)).await;
    assert_eq!(reports.load(Ordering::SeqCst), 0);
    // The machine "slept" for an hour between two ticks.
    slept.store(3600, Ordering::SeqCst);
    pass(Duration::from_secs(31)).await;
    assert_eq!(reports.load(Ordering::SeqCst), 1);
    pass(Duration::from_secs(5 * 60)).await;
    assert_eq!(reports.load(Ordering::SeqCst), 1);
    ticker.abort();
}

#[tokio::test(start_paused = true)]
async fn a_push_between_done_and_its_completion_wakes_sync_after_re_idle() {
    let (client, server) = session(IDLE_CAPS, |mut peer| async move {
        peer.examine().await;
        let tag = peer.enter_idle().await;
        peer.expect("DONE").await;
        peer.send(&format!("* 5 EXISTS\r\n{tag} OK IDLE terminated\r\n"))
            .await;
        let tag = peer.enter_idle().await;
        peer.leave_idle(&tag).await;
        peer.logout().await;
    })
    .await;
    let mut harness = Harness::start(vec![client]).await;
    pass(Duration::from_secs(10 * 60 + 5)).await;
    assert_eq!(
        harness.wakes(),
        2,
        "connect wake plus exactly one for the push"
    );
    pass(Duration::from_secs(30)).await;
    assert_eq!(harness.wakes(), 2);

    harness.control.stop.notify_one();
    (&mut harness.watcher).await.unwrap();
    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_push_before_the_new_idle_is_acknowledged_wakes_sync() {
    let (client, server) = session(IDLE_CAPS, |mut peer| async move {
        peer.examine().await;
        let tag = peer.enter_idle().await;
        peer.leave_idle(&tag).await;
        peer.expect("IDLE").await;
        peer.send("* 6 EXISTS\r\n+ idling\r\n").await;
        peer.expect("DONE").await;
    })
    .await;
    let harness = Harness::start(vec![client]).await;
    pass(Duration::from_secs(10 * 60 + 5)).await;
    assert_eq!(harness.wakes(), 2);
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn a_reconnect_signal_during_connect_does_not_bounce_the_new_session() {
    let (client, server) = session(IDLE_CAPS, |mut peer| async move {
        peer.examine().await;
        let tag = peer.enter_idle().await;
        peer.leave_idle(&tag).await;
        peer.logout().await;
    })
    .await;
    let session = Arc::new(Mutex::new(Some(client)));
    let connects = Arc::new(AtomicU32::new(0));
    let health = Arc::new(watch::channel(IdleHealth::Starting).0);
    let control = Arc::new(IdleControl::default());
    let sync_wake = Arc::new(Notify::new());
    let context = WatchContext {
        health: health.clone(),
        sync_wake,
        control: control.clone(),
        timing: Timing::default(),
    };
    let counter = connects.clone();
    let mut watcher = tokio::spawn(watch(
        move || {
            counter.fetch_add(1, Ordering::SeqCst);
            let next = session.lock().unwrap().take();
            async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                match next {
                    Some(session) => Ok(session),
                    None => std::future::pending().await,
                }
            }
        },
        context,
    ));
    pass(Duration::from_secs(1)).await;
    // The machine "slept" while the connection was still being opened.
    control.reconnect.notify_one();
    pass(Duration::from_secs(60)).await;
    assert_eq!(*health.borrow(), IdleHealth::Live);
    assert_eq!(connects.load(Ordering::SeqCst), 1);

    control.stop.notify_one();
    (&mut watcher).await.unwrap();
    server.await.unwrap();
}
