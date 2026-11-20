//! A Markup re-upload racing the blob GC: the upload writes and touches its blobs under the lock
//! `gc_sweep` holds, so an old unattached blob is never removed between `put` and `touch`.
//! Lives in its own test binary because `fail` failpoints are process-global.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, SystemTime};

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use serde_json::json;

const PNG: &[u8] = include_bytes!("../../../fixtures/markup/cross-out.png");
const STROKES: &str = include_str!("../../../fixtures/markup/cross-out.strokes.json");
const MD: &str = "# Plan\n\nWe ship on Friday.\n";
const GRACE: Duration = Duration::from_secs(3600);

// spec: S49-03
#[test]
fn a_reupload_is_not_swept_between_put_and_touch() {
    let ts = TestServer::start();
    let hash = sha256_hex(MD.as_bytes());
    let res = ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .content_type("application/octet-stream")
        .send(MD.as_bytes())
        .unwrap();
    assert!(res.status().is_success());
    let (status, doc) =
        ts.post("/api/v1/docs", Some(&ts.owner_token), &json!({ "vault": "notes", "name": "race", "md_hash": hash }));
    assert_eq!(status, 201, "{doc}");
    let doc = doc["id"].as_str().unwrap().to_string();
    let strokes = serde_json::to_vec(&serde_json::from_str::<serde_json::Value>(STROKES).unwrap()).unwrap();

    // An upload nobody attached, older than the grace period: the GC's to remove.
    let first = ts.store.upload_markup(&doc, PNG, &strokes).unwrap();
    let old = SystemTime::now() - 2 * GRACE;
    for h in [&first.markup.image, &first.markup.strokes] {
        std::fs::File::options().write(true).open(ts.store.blobs().path(h)).unwrap().set_modified(old).unwrap();
    }

    // Between the re-upload's `put` (the blob is there) and its `touch`, a sweep starts.
    let fired = Arc::new(AtomicBool::new(false));
    let swept: Arc<Mutex<Option<mpsc::Receiver<Vec<String>>>>> = Arc::default();
    let swept_during_upload = Arc::new(AtomicBool::new(false));
    {
        let (store, fired, swept, during) =
            (ts.store.clone(), fired.clone(), swept.clone(), swept_during_upload.clone());
        fail::cfg_callback("markup.between_put_and_touch", move || {
            if fired.swap(true, Ordering::SeqCst) {
                return;
            }
            let (tx, rx) = mpsc::channel();
            let store = store.clone();
            std::thread::spawn(move || tx.send(store.gc_blobs(GRACE).unwrap()).unwrap());
            // Unguarded, the sweep finishes here; under the lock it waits for the upload.
            if let Ok(removed) = rx.recv_timeout(Duration::from_millis(500)) {
                during.store(true, Ordering::SeqCst);
                let (tx, rx2) = mpsc::channel();
                tx.send(removed).unwrap();
                *swept.lock().unwrap() = Some(rx2);
            } else {
                *swept.lock().unwrap() = Some(rx);
            }
        })
        .unwrap();
    }
    let again = ts.store.upload_markup(&doc, PNG, &strokes);
    fail::remove("markup.between_put_and_touch");

    let again = again.expect("the re-upload succeeds");
    assert_eq!(again.id, first.id);
    assert!(fired.load(Ordering::SeqCst), "the failpoint ran");
    let removed = swept.lock().unwrap().take().unwrap().recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(!swept_during_upload.load(Ordering::SeqCst), "the sweep waited for the upload's lock");
    assert!(!removed.contains(&again.markup.image), "the sweep kept the fresh upload: {removed:?}");
    assert!(ts.store.blobs().has(&again.markup.image) && ts.store.blobs().has(&again.markup.strokes));
}
