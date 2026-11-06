//! Following a reply from another device: while a chat's reply is being written, everything
//! it streams is kept (from its start) and passed on, so the chat opened on a phone (or
//! another PC) shows it as it's written, including any request to allow a command, which can
//! be answered there.
//!
//! A follower gets everything so far, then the rest as it comes. Requests meant for the
//! device that started the reply (`file_request`: its own folders and tools) are passed on
//! too; followers ignore them.
//!
//! A device with the chat open also *watches* it: it's told when a reply starts there (sent
//! from another device), and follows that reply as above.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{LazyLock, Mutex};

use axum::response::sse::Event;
use futures_util::{Stream, StreamExt};
use tokio::sync::{broadcast, mpsc};

struct Feed {
    /// Everything streamed so far.
    backlog: Vec<Event>,
    tx: broadcast::Sender<Event>,
    /// Which reply (a new one in the same chat replaces it).
    run: u64,
}

static FEEDS: LazyLock<Mutex<HashMap<String, Feed>>> = LazyLock::new(Default::default);
static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Chats being watched: told (`()`) when a reply starts in them.
static BELLS: LazyLock<Mutex<HashMap<String, broadcast::Sender<()>>>> = LazyLock::new(Default::default);

fn bell(chat_id: &str) -> broadcast::Sender<()> {
    let mut bells = BELLS.lock().unwrap_or_else(|p| p.into_inner());
    // Nobody watching any more: forget them.
    bells.retain(|id, b| id == chat_id || b.receiver_count() > 0);
    bells.entry(chat_id.to_string()).or_insert_with(|| broadcast::channel(16).0).clone()
}

/// Watch a chat: a `replying` event now if a reply is being written there, then one each time
/// a reply starts (follow it with [`follow`]). Runs until the device closes it.
pub fn watch(chat_id: &str) -> impl Stream<Item = Result<Event, Infallible>> {
    let rx = bell(chat_id).subscribe();
    let now = active(chat_id);
    let replying = || Ok::<Event, Infallible>(Event::default().event("replying").data("{}"));
    let first = futures_util::stream::iter(now.then(replying));
    let rest = futures_util::stream::unfold(rx, move |mut rx| async move {
        match rx.recv().await {
            Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => Some((replying(), rx)),
            Err(broadcast::error::RecvError::Closed) => None,
        }
    });
    first.chain(rest)
}

/// Why a chat's last reply failed (for a routine, which has no screen to show the error on).
static FAILURES: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

pub fn set_failure(chat_id: &str, why: &str) {
    let mut f = FAILURES.lock().unwrap_or_else(|p| p.into_inner());
    if f.len() > 200 {
        f.clear();
    }
    f.insert(chat_id.to_string(), why.to_string());
}

/// Why this chat's last reply failed, if it did (and forget it).
pub fn take_failure(chat_id: &str) -> Option<String> {
    FAILURES.lock().unwrap_or_else(|p| p.into_inner()).remove(chat_id)
}

/// Most events kept for a follower joining late (a very long reply keeps its start and end).
const BACKLOG_MAX: usize = 50_000;

/// A reply starts in this chat: pass `rx` (what it streams) on to the device that asked, and
/// keep it for followers. Returns what the asking device reads.
pub fn tee(chat_id: &str, mut rx: mpsc::Receiver<Event>) -> mpsc::Receiver<Event> {
    let (out_tx, out_rx) = mpsc::channel::<Event>(256);
    let (tx, _) = broadcast::channel(1024);
    let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    FEEDS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(chat_id.to_string(), Feed { backlog: Vec::new(), tx: tx.clone(), run });
    // Devices with the chat open start following it.
    let _ = bell(chat_id).send(());
    let chat = chat_id.to_string();
    tokio::spawn(async move {
        let mut asker = Some(out_tx);
        while let Some(ev) = rx.recv().await {
            {
                let mut feeds = FEEDS.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(f) = feeds.get_mut(&chat).filter(|f| f.run == run) {
                    if f.backlog.len() < BACKLOG_MAX {
                        f.backlog.push(ev.clone());
                    }
                    let _ = f.tx.send(ev.clone());
                }
            }
            // The asking device may have gone (closed the app): the reply carries on.
            if let Some(a) = &asker {
                if a.send(ev).await.is_err() {
                    asker = None;
                }
            }
        }
        let mut feeds = FEEDS.lock().unwrap_or_else(|p| p.into_inner());
        if feeds.get(&chat).is_some_and(|f| f.run == run) {
            feeds.remove(&chat);
        }
    });
    out_rx
}

/// Is a reply being written in this chat now?
pub fn active(chat_id: &str) -> bool {
    FEEDS.lock().unwrap_or_else(|p| p.into_inner()).contains_key(chat_id)
}

/// How many replies are being written now (anywhere).
pub fn count() -> usize {
    FEEDS.lock().unwrap_or_else(|p| p.into_inner()).len()
}

/// Follow this chat's reply: everything so far, then the rest. `None` when none is running.
pub fn follow(chat_id: &str) -> Option<impl Stream<Item = Result<Event, Infallible>>> {
    let (backlog, sub) = {
        let feeds = FEEDS.lock().unwrap_or_else(|p| p.into_inner());
        let f = feeds.get(chat_id)?;
        // Subscribed under the lock, so nothing falls between the backlog and the rest.
        (f.backlog.clone(), f.tx.subscribe())
    };
    let rest = futures_util::stream::unfold(sub, |mut sub| async move {
        loop {
            match sub.recv().await {
                Ok(ev) => return Some((Ok::<Event, Infallible>(ev), sub)),
                // Fell behind: carry on from here (a token or two is lost on screen; the
                // saved reply arrives at the end anyway).
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Some(futures_util::stream::iter(backlog.into_iter().map(Ok::<Event, Infallible>)).chain(rest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn a_watcher_hears_when_a_reply_starts() {
        let chat = format!("c-{}", uuid::Uuid::new_v4());
        let mut w = Box::pin(watch(&chat));
        let (tx, rx) = mpsc::channel(8);
        let _asker = tee(&chat, rx);
        let got = tokio::time::timeout(std::time::Duration::from_secs(1), w.next()).await;
        assert!(matches!(got, Ok(Some(Ok(_)))), "told a reply started");
        drop(tx);
    }

    #[tokio::test]
    async fn a_follower_gets_the_start_then_the_rest() {
        let chat = format!("c-{}", uuid::Uuid::new_v4());
        let (tx, rx) = mpsc::channel(8);
        let mut asker = tee(&chat, rx);
        tx.send(Event::default().event("token").data("a")).await.unwrap();
        assert!(asker.recv().await.is_some());
        assert!(active(&chat));
        let follower = follow(&chat).expect("running");
        tx.send(Event::default().event("token").data("b")).await.unwrap();
        drop(tx);
        let got: Vec<_> = follower.collect().await;
        assert_eq!(got.len(), 2, "the start and the rest");
        // Finished: nothing to follow.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!active(&chat) && follow(&chat).is_none());
    }
}
