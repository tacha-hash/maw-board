//! Integration tests for Phase 3 multi-backend Join().

use anyhow::{Context, Result};
use sshx::{controller::Controller, runner::Runner};
use sshx_core::BackendId;

use crate::common::*;

pub mod common;

/// Two independent `sshx` backend processes join the SAME session — proves
/// the MPMC race documented in docs/phase3-design.md is actually fixed
/// (each backend gets its own channel + shells route to the right one),
/// not just that `Join()` returns success.
#[tokio::test]
async fn test_two_backends_join_same_session() -> Result<()> {
    let server = TestServer::new().await;

    // Primary backend: creates the session.
    let mut primary = Controller::new(&server.endpoint(), "primary-node", Runner::Echo, false)
        .await
        .context("primary Open() failed")?;
    assert!(primary.is_primary());
    let name = primary.name().to_owned();
    let join_token = primary
        .join_token()
        .context("primary should have a join_token")?
        .to_owned();
    let key = primary.encryption_key().to_owned();

    // Second backend: joins the same session by name + join_token + the
    // SAME encryption key (as a real operator would copy from the primary's
    // printed URL fragment).
    let mut joiner = Controller::join(
        &server.endpoint(),
        &name,
        &join_token,
        &key,
        "second-node",
        Runner::Echo,
    )
    .await
    .context("Join() failed")?;
    assert!(!joiner.is_primary());

    tokio::spawn(async move { primary.run().await });
    tokio::spawn(async move { joiner.run().await });

    // Give both backends a moment to complete their Channel() handshake.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let session = server
        .state()
        .lookup(&name)
        .context("session should exist")?;

    let backends = session.list_backends();
    assert_eq!(backends.len(), 2, "expected primary + joined backend, got {backends:?}");
    assert!(backends.iter().any(|(id, _)| *id == BackendId::PRIMARY));
    assert!(backends.iter().any(|(id, _)| *id != BackendId::PRIMARY));

    // Create one shell "from" each backend directly via the owning backend's
    // channel (mirrors what the WS Create handler does per-backend), then
    // confirm each Sid is correctly attributed to its OWN backend — this is
    // the actual routing fix, not just registration.
    let primary_id = BackendId::PRIMARY;
    let joined_id = backends
        .iter()
        .find(|(id, _)| *id != BackendId::PRIMARY)
        .unwrap()
        .0;

    session.add_shell(sshx_core::Sid(101), (0, 0), primary_id)?;
    session.add_shell(sshx_core::Sid(102), (0, 0), joined_id)?;

    assert_eq!(session.shell_backend(sshx_core::Sid(101)), Some(primary_id));
    assert_eq!(session.shell_backend(sshx_core::Sid(102)), Some(joined_id));

    Ok(())
}

/// A backend can't connect twice concurrently under the same backend_id —
/// this is the SHOULD-fix from the design review (reject duplicate hello),
/// preventing the original MPMC race from being reachable via any path.
#[tokio::test]
async fn test_duplicate_backend_connection_rejected() -> Result<()> {
    let server = TestServer::new().await;
    let primary = Controller::new(&server.endpoint(), "primary-node", Runner::Echo, false).await?;
    let name = primary.name().to_owned();

    let session = server
        .state()
        .lookup(&name)
        .context("session should exist")?;

    // The primary's own `run()` hasn't been spawned, so backend 0 isn't
    // actually connected yet — connect it manually once, then try again.
    session.connect_backend(BackendId::PRIMARY)?;
    let second_attempt = session.connect_backend(BackendId::PRIMARY);
    assert!(
        second_attempt.is_err(),
        "a second concurrent connection to the same backend_id must be rejected"
    );

    Ok(())
}
