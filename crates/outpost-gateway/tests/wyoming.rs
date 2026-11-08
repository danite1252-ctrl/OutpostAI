//! Home Assistant's voice protocol: a Wyoming conversation over an in-memory connection.

use outpost_gateway::wyoming::{encode, read_event, serve, Event};
use outpost_gateway::{AppState, Config};
use serde_json::json;
use tokio::io::{AsyncWriteExt, BufReader};

#[tokio::test]
async fn home_assistant_asks_what_we_offer_and_gets_an_answer_to_each_request() {
    let pool = outpost_core::db::open_in_memory().await.unwrap();
    let mut config = Config::local_default();
    config.data_dir = std::env::temp_dir().join(format!("outpost-wyoming-{}", uuid::Uuid::new_v4()));
    let state = AppState::with_pool(pool, config);

    let (ours, theirs) = tokio::io::duplex(1 << 20);
    let kept = state.clone();
    let server = tokio::spawn(async move { serve(&state, ours).await });
    let (r, mut w) = tokio::io::split(theirs);
    let mut r = BufReader::new(r);

    // describe → info, with speech-to-text offered.
    w.write_all(&encode(&Event::new("describe", json!({})))).await.unwrap();
    let info = read_event(&mut r).await.unwrap().unwrap();
    assert_eq!(info.kind, "info");
    assert_eq!(info.data["asr"][0]["models"][0]["name"], "whisper");

    // ping → pong.
    w.write_all(&encode(&Event::new("ping", json!({})))).await.unwrap();
    assert_eq!(read_event(&mut r).await.unwrap().unwrap().kind, "pong");

    // Speech: whatever the voice service says, Home Assistant gets a transcript or an error, never silence.
    w.write_all(&encode(&Event::new("transcribe", json!({ "language": "en-US" })))).await.unwrap();
    w.write_all(&encode(&Event::new("audio-start", json!({ "rate": 16000, "width": 2, "channels": 1 })))).await.unwrap();
    let mut chunk = Event::new("audio-chunk", json!({ "rate": 16000, "width": 2, "channels": 1 }));
    chunk.payload = vec![0u8; 3200];
    w.write_all(&encode(&chunk)).await.unwrap();
    w.write_all(&encode(&Event::new("audio-stop", json!({})))).await.unwrap();
    let heard = read_event(&mut r).await.unwrap().unwrap();
    assert!(heard.kind == "transcript" || heard.kind == "error", "{heard:?}");

    // A voice: audio back, or an error.
    w.write_all(&encode(&Event::new("synthesize", json!({ "text": "Hello", "voice": { "name": "af_heart" } })))).await.unwrap();
    let spoke = read_event(&mut r).await.unwrap().unwrap();
    assert!(spoke.kind == "audio-start" || spoke.kind == "error", "{spoke:?}");
    if spoke.kind == "audio-start" {
        // The voice service is running here: the audio, then its end.
        loop {
            let e = read_event(&mut r).await.unwrap().unwrap();
            if e.kind == "audio-stop" {
                break;
            }
            assert_eq!(e.kind, "audio-chunk");
            assert!(!e.payload.is_empty());
        }
    }

    // Said to the conversation agent with no persona chosen: not handled, with why.
    w.write_all(&encode(&Event::new("transcript", json!({ "text": "turn off the kitchen lamp", "context": { "conversation_id": "c1", "device_id": "phone-1" } }))))
        .await
        .unwrap();
    let handled = read_event(&mut r).await.unwrap().unwrap();
    assert_eq!(handled.kind, "not-handled");
    assert!(handled.data["text"].as_str().unwrap().contains("choose a persona"));
    // The device it came from is remembered, for linking to a person.
    let devices = outpost_gateway::wyoming::devices(&kept).await;
    assert_eq!(devices.len(), 1);
    assert_eq!((devices[0].id.as_str(), devices[0].link.is_none()), ("phone-1", true));

    // Closing ends it cleanly.
    drop(w);
    drop(r);
    let _ = server.await;
}
