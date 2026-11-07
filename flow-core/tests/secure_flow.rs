use argon_flow_core::{Config, FlowService};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("argon-flow-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        // All recursive cleanup stays in a uniquely created direct child of the explicit test temp directory.
        if let (Ok(root), Ok(temp)) = (self.0.canonicalize(), std::env::temp_dir().canonicalize()) {
            if root.parent() == Some(temp.as_path())
                && root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .starts_with("argon-flow-test-")
            {
                let _ = std::fs::remove_dir_all(root);
            }
        }
    }
}
async fn engine(root: &Root, name: &str) -> Arc<FlowService> {
    let mut config = Config::desktop(
        root.0.join(format!("{name}-state")),
        root.0.join(format!("{name}-received")),
        name.into(),
    );
    config.bind_address = "127.0.0.1".parse().unwrap();
    config.port = 0;
    config.discovery = false;
    let engine = FlowService::new(config, |_| {}).await.unwrap();
    engine.start().await.unwrap();
    engine
}
async fn until(mut predicate: impl AsyncPredicate) {
    predicate.wait().await;
}
trait AsyncPredicate {
    async fn wait(&mut self);
}
impl<F, Fut> AsyncPredicate for F
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    async fn wait(&mut self) {
        tokio::time::timeout(Duration::from_secs(20), async {
            while !self().await {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("operation timed out");
    }
}
async fn pair(a: &Arc<FlowService>, b: &Arc<FlowService>) {
    a.connect(&format!("127.0.0.1:{}", b.snapshot().await.port))
        .await
        .unwrap();
    until(|| async {
        !a.snapshot().await.pairings.is_empty() && !b.snapshot().await.pairings.is_empty()
    })
    .await;
    let left = a.snapshot().await.pairings.remove(0);
    let right = b.snapshot().await.pairings.remove(0);
    assert_eq!(left.code, right.code);
    assert_eq!(left.code.len(), 6);
    assert!(a.snapshot().await.peers.iter().all(|peer| !peer.trusted));
    a.confirm_pairing(&left.id, true).await.unwrap();
    b.confirm_pairing(&right.id, true).await.unwrap();
    until(|| async {
        a.snapshot().await.peers.iter().any(|p| p.trusted)
            && b.snapshot().await.peers.iter().any(|p| p.trusted)
    })
    .await;
}
async fn receive(b: &Arc<FlowService>) -> String {
    until(|| async { !b.snapshot().await.incoming.is_empty() }).await;
    let id = b.snapshot().await.incoming.remove(0).id;
    b.respond_incoming(&id, true).await.unwrap();
    id
}
async fn completed(engine: &Arc<FlowService>, id: &str) {
    // Debug-mode encryption on CI may take more than 20 seconds for the restart test's 64 MB file.
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            if engine.snapshot().await.transfers.iter().any(|t| {
                t.id == id && ["completed", "error", "cancelled"].contains(&t.status.as_str())
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("transfer completion timed out");
    let t = engine
        .snapshot()
        .await
        .transfers
        .into_iter()
        .find(|t| t.id == id)
        .unwrap();
    assert_eq!(t.status, "completed", "{:?}", t.error);
}

#[tokio::test]
async fn authenticated_pairing_requires_both_screens_and_persists_identity() {
    let root = Root::new();
    let a = engine(&root, "alice").await;
    let b = engine(&root, "bob").await;
    let identity = a.snapshot().await.id;
    pair(&a, &b).await;
    assert_eq!(
        a.snapshot().await.peers[0].address,
        format!("127.0.0.1:{}", b.snapshot().await.port)
    );
    assert_eq!(
        b.snapshot().await.peers[0].address,
        format!("127.0.0.1:{}", a.snapshot().await.port)
    );
    a.stop().await;
    b.stop().await;
    drop(a);
    let restored = engine(&root, "alice").await;
    assert_eq!(restored.snapshot().await.id, identity);
    assert!(restored.snapshot().await.peers[0].trusted);
    restored.stop().await;
    #[cfg(windows)]
    {
        let bytes = std::fs::read(root.0.join("alice-state/identity-v2.bin")).unwrap();
        assert!(serde_json::from_slice::<serde_json::Value>(&bytes).is_err());
    }
}

#[tokio::test]
async fn rejected_pairing_does_not_create_trust() {
    let root = Root::new();
    let a = engine(&root, "alice").await;
    let b = engine(&root, "bob").await;
    a.connect(&format!("127.0.0.1:{}", b.snapshot().await.port))
        .await
        .unwrap();
    until(|| async {
        !a.snapshot().await.pairings.is_empty() && !b.snapshot().await.pairings.is_empty()
    })
    .await;
    let x = a.snapshot().await.pairings[0].id.clone();
    let y = b.snapshot().await.pairings[0].id.clone();
    a.confirm_pairing(&x, true).await.unwrap();
    b.confirm_pairing(&y, false).await.unwrap();
    until(|| async { a.snapshot().await.last_error.is_some() }).await;
    assert!(a.snapshot().await.peers.iter().all(|p| !p.trusted));
    assert!(b.snapshot().await.peers.iter().all(|p| !p.trusted));
    a.stop().await;
    b.stop().await;
}

#[tokio::test]
async fn encrypted_files_empty_unicode_and_collisions_are_saved_and_acknowledged() {
    let root = Root::new();
    let a = engine(&root, "alice").await;
    let b = engine(&root, "bob").await;
    pair(&a, &b).await;
    for (index, name, body) in [
        (0, "تقرير المشروع.bin", vec![23u8; 2 * 1024 * 1024]),
        (1, "empty.txt", vec![]),
        (2, "تقرير المشروع.bin", vec![99u8; 128 * 1024]),
    ] {
        let path = root.0.join(format!("source-{index}")).join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &body).unwrap();
        let selected = a.select_files(vec![path]).await.unwrap();
        let id = a
            .send_files(&b.snapshot().await.id, vec![selected[0].id.clone()])
            .await
            .unwrap()
            .remove(0);
        let incoming = receive(&b).await;
        completed(&a, &id).await;
        completed(&b, &incoming).await;
        let saved = b
            .snapshot()
            .await
            .transfers
            .into_iter()
            .find(|t| t.id == incoming)
            .unwrap()
            .path
            .unwrap();
        assert_eq!(
            Sha256::digest(std::fs::read(saved).unwrap()),
            Sha256::digest(&body)
        );
    }
    assert!(root.0.join("bob-received/تقرير المشروع.bin").exists());
    assert!(root.0.join("bob-received/تقرير المشروع (1).bin").exists());
    a.stop().await;
    b.stop().await;
}

#[tokio::test]
async fn clipboard_requires_receive_consent_and_is_not_persisted() {
    let root = Root::new();
    let a = engine(&root, "alice").await;
    let b = engine(&root, "bob").await;
    pair(&a, &b).await;
    let text = "https://example.com/file?token=keep-exactly&part=2\nمرحبا";
    let id = a
        .send_text(&b.snapshot().await.id, text.into())
        .await
        .unwrap();
    until(|| async { !b.snapshot().await.incoming.is_empty() }).await;
    assert!(b.snapshot().await.clipboard.is_empty());
    receive(&b).await;
    completed(&a, &id).await;
    assert_eq!(b.snapshot().await.clipboard[0].text, text);
    a.stop().await;
    b.stop().await;
    let history = std::fs::read_to_string(root.0.join("bob-state/history-v2.json")).unwrap();
    assert!(!history.contains("keep-exactly"));
}

#[tokio::test]
async fn invalid_selection_remote_addresses_and_untrusted_sends_are_rejected() {
    let root = Root::new();
    let a = engine(&root, "alice").await;
    assert!(a.connect("8.8.8.8:54127").await.is_err());
    assert!(a.connect("0.0.0.0:0").await.is_err());
    assert!(a
        .send_files("unknown", vec!["arbitrary-path".into()])
        .await
        .is_err());
    assert!(a.select_files(vec![root.0.clone()]).await.is_err());
    a.stop().await;
}

#[tokio::test]
async fn declining_incoming_files_never_creates_a_completed_transfer() {
    let root = Root::new();
    let a = engine(&root, "alice").await;
    let b = engine(&root, "bob").await;
    pair(&a, &b).await;
    let path = root.0.join("denied.bin");
    std::fs::write(&path, b"must not be delivered").unwrap();
    let selected = a.select_files(vec![path]).await.unwrap();
    let id = a
        .send_files(&b.snapshot().await.id, vec![selected[0].id.clone()])
        .await
        .unwrap()
        .remove(0);
    until(|| async { !b.snapshot().await.incoming.is_empty() }).await;
    let request = b.snapshot().await.incoming[0].id.clone();
    b.respond_incoming(&request, false).await.unwrap();
    until(|| async {
        a.snapshot()
            .await
            .transfers
            .iter()
            .any(|t| t.id == id && t.status == "error")
    })
    .await;
    assert!(!root.0.join("bob-received/denied.bin").exists());
    a.stop().await;
    b.stop().await;
}

#[tokio::test]
async fn service_can_stop_and_restart_without_an_orphan_listener() {
    let root = Root::new();
    let a = engine(&root, "alice").await;
    let port = a.snapshot().await.port;
    a.stop().await;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    drop(listener);
    a.start().await.unwrap();
    assert!(a.snapshot().await.enabled);
    a.stop().await;
}

#[tokio::test]
async fn corrupt_resume_prefix_is_reset_before_delivery() {
    let root = Root::new();
    let a = engine(&root, "alice").await;
    let b = engine(&root, "bob").await;
    pair(&a, &b).await;
    let body = vec![91u8; 2 * 1024 * 1024];
    let hash: String = Sha256::digest(&body)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect();
    let partial_id: String = Sha256::digest(format!("{hash}\0resume.bin").as_bytes())
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect();
    let partial = root.0.join("bob-received/.argon-partials");
    std::fs::create_dir_all(&partial).unwrap();
    std::fs::write(
        partial.join(format!("{}-{partial_id}.part", a.snapshot().await.id)),
        vec![7u8; 512 * 1024],
    )
    .unwrap();
    let source = root.0.join("resume.bin");
    std::fs::write(&source, &body).unwrap();
    let selected = a.select_files(vec![source]).await.unwrap();
    let id = a
        .send_files(&b.snapshot().await.id, vec![selected[0].id.clone()])
        .await
        .unwrap()
        .remove(0);
    let incoming = receive(&b).await;
    completed(&a, &id).await;
    completed(&b, &incoming).await;
    assert_eq!(
        std::fs::read(root.0.join("bob-received/resume.bin")).unwrap(),
        body
    );
    a.stop().await;
    b.stop().await;
}

#[tokio::test]
async fn large_files_are_verified_even_above_one_hundred_megabytes() {
    let root = Root::new();
    let a = engine(&root, "alice").await;
    let b = engine(&root, "bob").await;
    pair(&a, &b).await;
    let path = root.0.join("large.bin");
    let source = std::fs::File::create(&path).unwrap();
    source.set_len(101 * 1024 * 1024).unwrap();
    drop(source);
    let selected = a.select_files(vec![path]).await.unwrap();
    let id = a
        .send_files(&b.snapshot().await.id, vec![selected[0].id.clone()])
        .await
        .unwrap()
        .remove(0);
    // Full-file hashing is visible before the receiver is asked for consent.
    let request = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let state = b.snapshot().await;
            if let Some(request) = state.incoming.first() {
                break request.clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    b.respond_incoming(&request.id, true).await.unwrap();
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let state = a.snapshot().await;
            let t = state.transfers.iter().find(|t| t.id == id).unwrap();
            if ["completed", "error", "cancelled"].contains(&t.status.as_str()) {
                assert_eq!(t.status, "completed", "{:?}", t.error);
                assert_eq!(t.sha256.as_ref().unwrap().len(), 64);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::metadata(root.0.join("bob-received/large.bin"))
            .unwrap()
            .len(),
        101 * 1024 * 1024
    );
    a.stop().await;
    b.stop().await;
}

#[tokio::test]
async fn revoked_devices_cannot_start_new_transfers() {
    let root = Root::new();
    let a = engine(&root, "alice").await;
    let b = engine(&root, "bob").await;
    pair(&a, &b).await;
    a.revoke(&b.snapshot().await.id).await.unwrap();
    assert!(a
        .send_text(&b.snapshot().await.id, "not allowed".into())
        .await
        .is_err());
    a.stop().await;
    b.stop().await;
}

#[tokio::test]
async fn interrupted_transfer_resumes_after_receiver_restart() {
    let root = Root::new();
    let a = engine(&root, "alice").await;
    let b = engine(&root, "bob").await;
    pair(&a, &b).await;
    let peer_id = b.snapshot().await.id;
    let source = root.0.join("interrupted.bin");
    let file = std::fs::File::create(&source).unwrap();
    file.set_len(64 * 1024 * 1024).unwrap();
    drop(file);
    let selected = a.select_files(vec![source.clone()]).await.unwrap();
    let first = a
        .send_files(&peer_id, vec![selected[0].id.clone()])
        .await
        .unwrap()
        .remove(0);
    let incoming = receive(&b).await;
    until(|| async {
        b.snapshot()
            .await
            .transfers
            .iter()
            .any(|t| t.id == incoming && t.bytes > 0 && t.bytes < t.total)
    })
    .await;
    a.cancel(&first).await.unwrap();
    until(|| async {
        a.snapshot()
            .await
            .transfers
            .iter()
            .any(|t| t.id == first && t.status == "cancelled")
    })
    .await;
    b.stop().await;
    drop(b);
    let partials = root.0.join("bob-received/.argon-partials");
    let entries: Vec<_> = std::fs::read_dir(&partials)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(entries.len(), 1);
    let saved_prefix = std::fs::metadata(&entries[0]).unwrap().len();
    assert!(saved_prefix > 0 && saved_prefix < 64 * 1024 * 1024);
    let b = engine(&root, "bob").await;
    assert_eq!(b.snapshot().await.id, peer_id);
    assert_eq!(std::fs::metadata(&entries[0]).unwrap().len(), saved_prefix);
    a.connect(&format!("127.0.0.1:{}", b.snapshot().await.port))
        .await
        .unwrap();
    until(|| async {
        let port = b.snapshot().await.port;
        a.snapshot()
            .await
            .peers
            .iter()
            .any(|p| p.id == peer_id && p.online && p.address.ends_with(&format!(":{port}")))
    })
    .await;
    let second = a
        .send_files(&peer_id, vec![selected[0].id.clone()])
        .await
        .unwrap()
        .remove(0);
    let incoming = receive(&b).await;
    completed(&a, &second).await;
    completed(&b, &incoming).await;
    assert_eq!(
        Sha256::digest(std::fs::read(&source).unwrap()),
        Sha256::digest(std::fs::read(root.0.join("bob-received/interrupted.bin")).unwrap())
    );
    assert!(!entries[0].exists());
    a.stop().await;
    b.stop().await;
}

#[tokio::test]
async fn one_sided_revocation_requires_both_devices_to_pair_again() {
    let root = Root::new();
    let a = engine(&root, "alice").await;
    let b = engine(&root, "bob").await;
    pair(&a, &b).await;
    b.revoke(&a.snapshot().await.id).await.unwrap();
    a.connect(&format!("127.0.0.1:{}", b.snapshot().await.port))
        .await
        .unwrap();
    until(|| async {
        !a.snapshot().await.pairings.is_empty() && !b.snapshot().await.pairings.is_empty()
    })
    .await;
    let left = a.snapshot().await.pairings.remove(0);
    let right = b.snapshot().await.pairings.remove(0);
    assert_eq!(left.code, right.code);
    a.confirm_pairing(&left.id, true).await.unwrap();
    b.confirm_pairing(&right.id, true).await.unwrap();
    until(|| async { b.snapshot().await.peers.iter().any(|p| p.trusted) }).await;
    let id = a
        .send_text(&b.snapshot().await.id, "paired again".into())
        .await
        .unwrap();
    receive(&b).await;
    completed(&a, &id).await;
    a.stop().await;
    b.stop().await;
}
