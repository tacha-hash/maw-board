use anyhow::{Context, Result};
use sshx::{controller::Controller, encrypt::Encrypt, runner::Runner};
use sshx_core::{
    proto::{server_update::ServerMessage, NewShell, TerminalInput},
    BackendId, Sid, Uid,
};
use sshx_server::{
    web::protocol::{WsClient, WsWinsize},
    ServerOptions,
};
use tokio::time::{self, Duration};

use crate::common::*;

pub mod common;

#[tokio::test]
async fn test_handshake() -> Result<()> {
    let server = TestServer::new().await;
    let controller = Controller::new(&server.endpoint(), "", Runner::Echo, false).await?;
    controller.close().await?;
    Ok(())
}

#[tokio::test]
async fn test_command() -> Result<()> {
    let server = TestServer::new().await;
    let runner = Runner::Shell("/bin/bash".into());
    let mut controller = Controller::new(&server.endpoint(), "", runner, false).await?;

    let session = server
        .state()
        .lookup(controller.name())
        .context("couldn't find session in server state")?;

    let updates = session
        .backend_sender(BackendId::PRIMARY)
        .context("primary backend not registered")?;
    let new_shell = NewShell { id: 1, x: 0, y: 0 };
    updates.send(ServerMessage::CreateShell(new_shell)).await?;

    let key = controller.encryption_key();
    let encrypt = Encrypt::new(key);
    let offset = 4242;
    let data = TerminalInput {
        id: 1,
        data: encrypt.segment(0x200000000, offset, b"ls\r\n").into(),
        offset,
    };
    updates.send(ServerMessage::Input(data)).await?;

    tokio::select! {
        _ = controller.run() => (),
        _ = time::sleep(Duration::from_millis(1000)) => (),
    };
    controller.close().await?;
    Ok(())
}

#[tokio::test]
async fn test_ws_missing() -> Result<()> {
    let server = TestServer::new().await;

    let bad_endpoint = format!("ws://{}/not/an/endpoint", server.local_addr());
    assert!(ClientSocket::connect(&bad_endpoint, "", None, None)
        .await
        .is_err());

    // Unauthenticated (no session cookie) → the VR5 connect gate rejects the
    // upgrade before it ever reaches the session lookup.
    assert!(
        ClientSocket::connect(&server.ws_endpoint("foobar"), "", None, None)
            .await
            .is_err()
    );

    // Authenticated member of the (nonexistent) board → past the gate, then the
    // session-not-found close.
    let cookie = server.member_cookie("u", "foobar").await;
    let mut s =
        ClientSocket::connect(&server.ws_endpoint("foobar"), "", None, Some(&cookie)).await?;
    s.expect_close(4404).await;

    Ok(())
}

#[tokio::test]
async fn test_ws_basic() -> Result<()> {
    let server = TestServer::new().await;

    let mut controller = Controller::new(&server.endpoint(), "", Runner::Echo, false).await?;
    let name = controller.name().to_owned();
    let key = controller.encryption_key().to_owned();
    tokio::spawn(async move { controller.run().await });

    let cookie = server.member_cookie("u", &name).await;
    let mut s =
        ClientSocket::connect(&server.ws_endpoint(&name), &key, None, Some(&cookie)).await?;
    s.flush().await;
    assert_eq!(s.user_id, Uid(1));

    s.send(WsClient::Create(0, 0)).await;
    s.flush().await;
    assert_eq!(s.shells.len(), 1);
    assert!(s.shells.contains_key(&Sid(1)));

    s.send(WsClient::Subscribe(Sid(1), 0)).await;
    assert_eq!(s.read(Sid(1)), "");

    s.send_input(Sid(1), b"hello!").await;
    s.flush().await;
    assert_eq!(s.read(Sid(1)), "hello!");

    s.send_input(Sid(1), b" 123").await;
    s.flush().await;
    assert_eq!(s.read(Sid(1)), "hello! 123");

    Ok(())
}

#[tokio::test]
async fn test_ws_resize() -> Result<()> {
    let server = TestServer::new().await;

    let mut controller = Controller::new(&server.endpoint(), "", Runner::Echo, false).await?;
    let name = controller.name().to_owned();
    let key = controller.encryption_key().to_owned();
    tokio::spawn(async move { controller.run().await });

    let cookie = server.member_cookie("u", &name).await;
    let mut s =
        ClientSocket::connect(&server.ws_endpoint(&name), &key, None, Some(&cookie)).await?;

    s.send(WsClient::Move(Sid(1), None)).await; // error: does not exist yet!
    s.flush().await;
    assert_eq!(s.errors.len(), 1);

    s.send(WsClient::Create(0, 0)).await;
    s.flush().await;
    assert_eq!(s.shells.len(), 1);
    assert_eq!(*s.shells.get(&Sid(1)).unwrap(), WsWinsize::default());

    let new_size = WsWinsize {
        x: 42,
        y: 105,
        rows: 200,
        cols: 20,
        backend_id: 0,
    };
    s.send(WsClient::Move(Sid(1), Some(new_size))).await;
    s.send(WsClient::Move(Sid(2), Some(new_size))).await; // error: does not exist
    s.flush().await;
    assert_eq!(s.shells.len(), 1);
    assert_eq!(*s.shells.get(&Sid(1)).unwrap(), new_size);
    assert_eq!(s.errors.len(), 2);

    s.send(WsClient::Close(Sid(1))).await;
    s.flush().await;
    assert_eq!(s.shells.len(), 0);

    s.send(WsClient::Move(Sid(1), None)).await; // error: shell was closed
    s.flush().await;
    assert_eq!(s.errors.len(), 3);

    Ok(())
}

#[tokio::test]
async fn test_users_join() -> Result<()> {
    let server = TestServer::new().await;

    let mut controller = Controller::new(&server.endpoint(), "", Runner::Echo, false).await?;
    let name = controller.name().to_owned();
    let key = controller.encryption_key().to_owned();
    tokio::spawn(async move { controller.run().await });

    let endpoint = server.ws_endpoint(&name);
    let cookie = server.member_cookie("u", &name).await;
    let mut s1 = ClientSocket::connect(&endpoint, &key, None, Some(&cookie)).await?;
    s1.flush().await;
    assert_eq!(s1.users.len(), 1);

    let mut s2 = ClientSocket::connect(&endpoint, &key, None, Some(&cookie)).await?;
    s2.flush().await;
    assert_eq!(s2.users.len(), 2);

    drop(s2);
    let mut s3 = ClientSocket::connect(&endpoint, &key, None, Some(&cookie)).await?;
    s3.flush().await;
    assert_eq!(s3.users.len(), 2);

    s1.flush().await;
    assert_eq!(s1.users.len(), 2);

    Ok(())
}

#[tokio::test]
async fn test_users_metadata() -> Result<()> {
    let server = TestServer::new().await;

    let mut controller = Controller::new(&server.endpoint(), "", Runner::Echo, false).await?;
    let name = controller.name().to_owned();
    let key = controller.encryption_key().to_owned();
    tokio::spawn(async move { controller.run().await });

    let endpoint = server.ws_endpoint(&name);
    let cookie = server.member_cookie("u", &name).await;
    let mut s = ClientSocket::connect(&endpoint, &key, None, Some(&cookie)).await?;
    s.flush().await;
    assert_eq!(s.users.len(), 1);
    assert_eq!(s.users.get(&s.user_id).unwrap().cursor, None);

    s.send(WsClient::SetName("mr. foo".into())).await;
    s.send(WsClient::SetCursor(Some((40, 524)))).await;
    s.flush().await;
    let user = s.users.get(&s.user_id).unwrap();
    assert_eq!(user.name, "mr. foo");
    assert_eq!(user.cursor, Some((40, 524)));

    Ok(())
}

#[tokio::test]
async fn test_chat_messages() -> Result<()> {
    let server = TestServer::new().await;

    let mut controller = Controller::new(&server.endpoint(), "", Runner::Echo, false).await?;
    let name = controller.name().to_owned();
    let key = controller.encryption_key().to_owned();
    tokio::spawn(async move { controller.run().await });

    let endpoint = server.ws_endpoint(&name);
    let cookie = server.member_cookie("u", &name).await;
    let mut s1 = ClientSocket::connect(&endpoint, &key, None, Some(&cookie)).await?;
    let mut s2 = ClientSocket::connect(&endpoint, &key, None, Some(&cookie)).await?;

    s1.send(WsClient::SetName("billy".into())).await;
    s1.send(WsClient::Chat("hello there!".into())).await;
    s1.flush().await;

    s2.flush().await;
    assert_eq!(s2.messages.len(), 1);
    assert_eq!(
        s2.messages[0],
        (s1.user_id, "billy".into(), "hello there!".into())
    );

    let mut s3 = ClientSocket::connect(&endpoint, &key, None, Some(&cookie)).await?;
    s3.flush().await;
    assert_eq!(s1.messages.len(), 1);
    assert_eq!(s3.messages.len(), 0);

    Ok(())
}

#[tokio::test]
async fn test_read_write_permissions() -> Result<()> {
    let server = TestServer::new().await;

    // create controller with read-only mode enabled
    let mut controller = Controller::new(&server.endpoint(), "", Runner::Echo, true).await?;
    let name = controller.name().to_owned();
    let key = controller.encryption_key().to_owned();
    let write_url = controller
        .write_url()
        .expect("Should have write URL when enable_readers is true")
        .to_string();

    tokio::spawn(async move { controller.run().await });

    let write_password = write_url
        .split(',')
        .nth(1)
        .expect("Write URL should contain password");

    let cookie = server.member_cookie("u", &name).await;
    // connect with write access
    let mut writer = ClientSocket::connect(
        &server.ws_endpoint(&name),
        &key,
        Some(write_password),
        Some(&cookie),
    )
    .await?;
    writer.flush().await;

    // test write permissions
    writer.send(WsClient::Create(0, 0)).await;
    writer.flush().await;
    assert_eq!(
        writer.shells.len(),
        1,
        "Writer should be able to create a shell"
    );
    assert!(writer.errors.is_empty(), "Writer should not receive errors");

    // connect with read-only access
    let mut reader =
        ClientSocket::connect(&server.ws_endpoint(&name), &key, None, Some(&cookie)).await?;
    reader.flush().await;

    // test read-only restrictions
    reader.send(WsClient::Create(0, 0)).await;
    reader.flush().await;
    assert!(
        !reader.errors.is_empty(),
        "Reader should receive an error when attempting to create shell"
    );
    assert_eq!(
        reader.shells.len(),
        1,
        "Reader should still see the existing shell"
    );

    Ok(())
}

/// VR5 per-shell ownership (the exposure-gate negative test): a member of a
/// board who is NOT the account owning a shell's backend cannot type into that
/// shell — the server rejects `WsClient::Data` at the protocol layer, and the
/// input is never applied. The owner, meanwhile, types freely.
#[tokio::test]
async fn test_cross_account_shell_write_rejected() -> Result<()> {
    let mut options = ServerOptions::default();
    options.insecure_cookies = true; // plain-HTTP test login
    let server = TestServer::new_with_options(options).await;
    let state = server.state();

    // Accounts: alice owns a board + its backend; bob is a member.
    let a_hash = sshx_server::auth::hash_account_password("pw-alice").unwrap();
    let alice = state.accounts().bootstrap_account("alice", &a_hash).await?;
    let b_hash = sshx_server::auth::hash_account_password("pw-bob").unwrap();
    let bob = state.accounts().bootstrap_account("bob", &b_hash).await?;

    // Alice's connector token (what her backend authenticates with).
    let alice_token = "alice-connector-token-abc123";
    state
        .accounts()
        .set_connector_token("alice", &sshx_server::auth::connector_token_hash(alice_token))
        .await?;

    // Alice creates a board over HTTP (owner=alice, no backend yet).
    let base = format!("http://{}", server.local_addr());
    let http = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    http.post(format!("{base}/login"))
        .form(&[("username", "alice"), ("password", "pw-alice")])
        .send()
        .await?;
    let nb: serde_json::Value = http
        .post(format!("{base}/api/boards/new"))
        .json(&serde_json::json!({ "name": "neg" }))
        .send()
        .await?
        .json()
        .await?;
    let name = nb["name"].as_str().unwrap().to_string();
    let key = nb["key"].as_str().unwrap().to_string();
    let join_token = nb["join_token"].as_str().unwrap().to_string();

    // Alice's backend joins with her connector token → the server records the
    // backend (and thus its shells) as owned by alice.
    let mut backend = Controller::join(
        &server.endpoint(),
        &name,
        &join_token,
        &key,
        "alice-backend",
        Some(alice_token),
        Runner::Echo,
    )
    .await
    .context("alice backend join failed")?;
    tokio::spawn(async move { backend.run().await });

    // Bob is a member of the board (can view, per the connect gate).
    assert!(state.accounts().add_member_by_username(&name, "bob").await?);

    let alice_cookie = format!(
        "{}={}",
        sshx_server::auth::SESSION_COOKIE,
        sshx_server::auth::mint_session_cookie(state.mac(), &alice.id, 3600)
    );
    let bob_cookie = format!(
        "{}={}",
        sshx_server::auth::SESSION_COOKIE,
        sshx_server::auth::mint_session_cookie(state.mac(), &bob.id, 3600)
    );

    // Alice (owner) creates a shell on her backend and types into it — allowed.
    let mut sa =
        ClientSocket::connect(&server.ws_endpoint(&name), &key, None, Some(&alice_cookie)).await?;
    sa.send(WsClient::Create(0, 0)).await;
    sa.flush().await;
    assert_eq!(sa.shells.len(), 1, "alice created a shell");
    sa.send(WsClient::Subscribe(Sid(1), 0)).await;
    sa.send_input(Sid(1), b"hi from alice").await;
    sa.flush().await;
    assert_eq!(sa.read(Sid(1)), "hi from alice", "owner may type");
    assert!(sa.errors.is_empty(), "owner should get no error");

    // Bob (member, not the shell's owner) tries to type into alice's shell.
    let mut sb =
        ClientSocket::connect(&server.ws_endpoint(&name), &key, None, Some(&bob_cookie)).await?;
    sb.flush().await;
    assert!(sb.shells.contains_key(&Sid(1)), "bob can VIEW the shell");
    sb.send_input(Sid(1), b"evil from bob").await;
    sb.flush().await;
    assert!(
        sb.errors.iter().any(|e| e.contains("not your terminal")),
        "bob's write into alice's shell must be rejected, got errors: {:?}",
        sb.errors
    );

    // And bob's input must NOT have reached the terminal — alice's view is
    // unchanged (no "evil from bob" echoed back).
    sa.flush().await;
    assert_eq!(
        sa.read(Sid(1)),
        "hi from alice",
        "rejected input must not be applied to the shell"
    );

    Ok(())
}

/// The file browser is now behind the session gate: an unauthenticated request
/// is rejected before it reaches the handler.
#[tokio::test]
async fn test_file_api_requires_auth() -> Result<()> {
    let server = TestServer::new().await;
    let base = format!("http://{}", server.local_addr());
    let client = reqwest::Client::new();
    for path in ["/api/files?path=", "/api/file?path=x.txt"] {
        let resp = client.get(format!("{base}{path}")).send().await?;
        assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED, "{path}");
    }
    Ok(())
}

// Exercises the file-browser handlers (listing, traversal/symlink guards, size
// limits) as an authenticated user. Ignored by default because it reads and
// writes under /root/maw-workspace, which only exists on the server host; run
// with `cargo test -- --ignored` there.
#[ignore = "requires /root/maw-workspace (server host only)"]
#[tokio::test]
async fn test_files_api() -> Result<()> {
    // The file-browser API is now behind the session gate, so authenticate
    // first: create an account, log in (cookie stored by the client), then
    // every subsequent request carries the session automatically.
    let mut options = ServerOptions::default();
    options.insecure_cookies = true; // plain-HTTP test → cookie must omit Secure
    let server = TestServer::new_with_options(options).await;
    let hash = sshx_server::auth::hash_account_password("test-password-123").unwrap();
    server
        .state()
        .accounts()
        .bootstrap_account("tester", &hash)
        .await
        .unwrap();
    let client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let base = format!("http://{}", server.local_addr());
    let login = client
        .post(format!("{base}/login"))
        .form(&[("username", "tester"), ("password", "test-password-123")])
        .send()
        .await?;
    assert_eq!(login.status(), http::StatusCode::SEE_OTHER);
    // A cookie-less client is rejected by the gate, proving the endpoint is
    // protected now.
    let anon = reqwest::Client::new();
    let anon_resp = anon.get(format!("{base}/api/files?path=")).send().await?;
    assert_eq!(anon_resp.status(), http::StatusCode::UNAUTHORIZED);

    let test_prefix = format!("sshx_test_{}", server.local_addr().port());

    // 1. Test listing files
    let url_list = format!("http://{}/api/files?path=", server.local_addr());
    let resp = client.get(&url_list).send().await?;
    assert_eq!(resp.status(), http::StatusCode::OK);
    let text = resp.text().await?;
    assert!(text.contains("\"path\":"));
    assert!(text.contains("\"items\":"));

    // Create a temporary file inside /root/maw-workspace to test reading
    let temp_file_path = std::path::Path::new("/root/maw-workspace/test_hello.txt");
    tokio::fs::write(&temp_file_path, b"hello workspace file!").await?;

    // 2. Test reading the file
    let url_read = format!(
        "http://{}/api/file?path=test_hello.txt",
        server.local_addr()
    );
    let resp = client.get(&url_read).send().await?;
    assert_eq!(resp.status(), http::StatusCode::OK);
    assert_eq!(
        resp.headers().get(http::header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    assert_eq!(
        resp.headers().get(http::header::CACHE_CONTROL).unwrap(),
        "no-cache"
    );
    let json_text = resp.text().await?;
    assert!(json_text.contains("\"path\":\"test_hello.txt\""));
    assert!(json_text.contains("\"content\":\"hello workspace file!\""));

    // Clean up
    tokio::fs::remove_file(&temp_file_path).await.ok();

    // 3. Test reading nonexistent file
    let url_missing = format!(
        "http://{}/api/file?path=does_not_exist.txt",
        server.local_addr()
    );
    let resp = client.get(&url_missing).send().await?;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);

    // 4. Test directory traversal attempt
    let url_traversal = format!("http://{}/api/file?path=../etc/passwd", server.local_addr());
    let resp = client.get(&url_traversal).send().await?;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);

    // 5. Test symlink escape rejection: a harmless-looking name inside the root
    // must not serve a target outside FILES_ROOT.
    let symlink_escape_name = format!("{}_symlink_escape.txt", test_prefix);
    let symlink_escape_path =
        std::path::PathBuf::from(format!("/root/maw-workspace/{symlink_escape_name}"));
    tokio::fs::remove_file(&symlink_escape_path).await.ok();
    std::os::unix::fs::symlink("/etc/passwd", &symlink_escape_path)?;
    let url_symlink_escape = format!(
        "http://{}/api/file?path={}",
        server.local_addr(),
        symlink_escape_name
    );
    let resp = client.get(&url_symlink_escape).send().await?;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
    tokio::fs::remove_file(&symlink_escape_path).await.ok();

    // 6. Test resolved-path credential guard: a public symlink to a restricted
    // file inside the root must still be blocked after canonicalization.
    let restricted_target_name = format!("{}_secret_target.txt", test_prefix);
    let restricted_link_name = format!("{}_public_link.txt", test_prefix);
    let restricted_target =
        std::path::PathBuf::from(format!("/root/maw-workspace/{restricted_target_name}"));
    let restricted_link =
        std::path::PathBuf::from(format!("/root/maw-workspace/{restricted_link_name}"));
    tokio::fs::remove_file(&restricted_link).await.ok();
    tokio::fs::write(&restricted_target, b"secret through symlink").await?;
    std::os::unix::fs::symlink(&restricted_target, &restricted_link)?;
    let url_restricted_link = format!(
        "http://{}/api/file?path={}",
        server.local_addr(),
        restricted_link_name
    );
    let resp = client.get(&url_restricted_link).send().await?;
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
    tokio::fs::remove_file(&restricted_link).await.ok();
    tokio::fs::remove_file(&restricted_target).await.ok();

    // 7. Test symlinked directory escape rejection on /api/files.
    let symlink_dir_name = format!("{}_symlink_dir", test_prefix);
    let symlink_dir_path =
        std::path::PathBuf::from(format!("/root/maw-workspace/{symlink_dir_name}"));
    tokio::fs::remove_file(&symlink_dir_path).await.ok();
    std::os::unix::fs::symlink("/etc", &symlink_dir_path)?;
    let url_symlink_dir = format!(
        "http://{}/api/files?path={}",
        server.local_addr(),
        symlink_dir_name
    );
    let resp = client.get(&url_symlink_dir).send().await?;
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
    tokio::fs::remove_file(&symlink_dir_path).await.ok();

    // 8. Test dotfile rejection
    let dotfile_path = std::path::Path::new("/root/maw-workspace/.test_dotfile");
    tokio::fs::write(&dotfile_path, b"hidden content").await?;
    let url_dotfile = format!("http://{}/api/file?path=.test_dotfile", server.local_addr());
    let resp = client.get(&url_dotfile).send().await?;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
    tokio::fs::remove_file(&dotfile_path).await.ok();

    // 9. Test directory rejection
    let temp_dir_path = std::path::Path::new("/root/maw-workspace/test_dir");
    tokio::fs::create_dir(&temp_dir_path).await?;
    let url_dir = format!("http://{}/api/file?path=test_dir", server.local_addr());
    let resp = client.get(&url_dir).send().await?;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
    tokio::fs::remove_dir(&temp_dir_path).await.ok();

    // 10. Test binary file rejection (invalid UTF-8 bytes)
    let binary_path = std::path::Path::new("/root/maw-workspace/test_binary.bin");
    tokio::fs::write(&binary_path, b"hello \xff\xff world").await?;
    let url_binary = format!(
        "http://{}/api/file?path=test_binary.bin",
        server.local_addr()
    );
    let resp = client.get(&url_binary).send().await?;
    assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
    assert_eq!(resp.text().await?, "binary file");
    tokio::fs::remove_file(&binary_path).await.ok();

    // 11. Test file size > 1 MiB -> 413 Payload Too Large
    let large_path = std::path::Path::new("/root/maw-workspace/test_large.txt");
    let large_data = vec![b'a'; 1024 * 1024 + 10];
    tokio::fs::write(&large_path, &large_data).await?;
    let url_large = format!(
        "http://{}/api/file?path=test_large.txt",
        server.local_addr()
    );
    let resp = client.get(&url_large).send().await?;
    assert_eq!(resp.status(), http::StatusCode::PAYLOAD_TOO_LARGE);
    tokio::fs::remove_file(&large_path).await.ok();

    Ok(())
}

#[tokio::test]
async fn test_healthz() -> Result<()> {
    let server = TestServer::new().await;
    let client = reqwest::Client::new();
    let base = format!("http://{}", server.local_addr());

    // /api/healthz is a public path — reachable without a session.
    let resp = client.get(format!("{base}/api/healthz")).send().await?;
    assert_eq!(resp.status(), http::StatusCode::OK);
    assert_eq!(resp.text().await?, "OK");

    let resp = client
        .get(format!("{base}/api/healthz"))
        .header(http::header::HOST, "external.domain.com")
        .send()
        .await?;
    assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);

    Ok(())
}
