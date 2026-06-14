use anyhow::{Context, Result};
use sshx::{controller::Controller, encrypt::Encrypt, runner::Runner};
use sshx_core::{
    proto::{server_update::ServerMessage, NewShell, TerminalInput},
    Sid, Uid,
};
use sshx_server::{
    web::protocol::{WsClient, WsWinsize},
    ServerOptions,
};
use tokio::time::{self, Duration};

use crate::common::*;

pub mod common;

fn auth_cookie_from_response(resp: &reqwest::Response) -> String {
    resp.headers()
        .get(http::header::SET_COOKIE)
        .expect("login should set auth cookie")
        .to_str()
        .expect("set-cookie should be ASCII")
        .split(';')
        .next()
        .expect("set-cookie should contain a cookie pair")
        .to_string()
}

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

    let updates = session.update_tx();
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
    assert!(ClientSocket::connect(&bad_endpoint, "", None)
        .await
        .is_err());

    let mut s = ClientSocket::connect(&server.ws_endpoint("foobar"), "", None).await?;
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

    let mut s = ClientSocket::connect(&server.ws_endpoint(&name), &key, None).await?;
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

    let mut s = ClientSocket::connect(&server.ws_endpoint(&name), &key, None).await?;

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
    let mut s1 = ClientSocket::connect(&endpoint, &key, None).await?;
    s1.flush().await;
    assert_eq!(s1.users.len(), 1);

    let mut s2 = ClientSocket::connect(&endpoint, &key, None).await?;
    s2.flush().await;
    assert_eq!(s2.users.len(), 2);

    drop(s2);
    let mut s3 = ClientSocket::connect(&endpoint, &key, None).await?;
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
    let mut s = ClientSocket::connect(&endpoint, &key, None).await?;
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
    let mut s1 = ClientSocket::connect(&endpoint, &key, None).await?;
    let mut s2 = ClientSocket::connect(&endpoint, &key, None).await?;

    s1.send(WsClient::SetName("billy".into())).await;
    s1.send(WsClient::Chat("hello there!".into())).await;
    s1.flush().await;

    s2.flush().await;
    assert_eq!(s2.messages.len(), 1);
    assert_eq!(
        s2.messages[0],
        (s1.user_id, "billy".into(), "hello there!".into())
    );

    let mut s3 = ClientSocket::connect(&endpoint, &key, None).await?;
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

    // connect with write access
    let mut writer =
        ClientSocket::connect(&server.ws_endpoint(&name), &key, Some(write_password)).await?;
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
    let mut reader = ClientSocket::connect(&server.ws_endpoint(&name), &key, None).await?;
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

#[tokio::test]
async fn test_files_api() -> Result<()> {
    let server = TestServer::new().await;
    let client = reqwest::Client::new();
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
async fn test_board_password_gate() -> Result<()> {
    let mut options = ServerOptions::default();
    options.board_password = Some("test board password".to_string());
    let server = TestServer::new_with_options(options).await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let base = format!("http://{}", server.local_addr());

    // Browser-facing board routes redirect to the local login page.
    let resp = client.get(format!("{base}/go")).send().await?;
    assert_eq!(resp.status(), http::StatusCode::SEE_OTHER);
    assert_eq!(
        resp.headers().get(http::header::LOCATION).unwrap(),
        "/login?next=%2Fgo"
    );

    let resp = client.get(format!("{base}/s/demo")).send().await?;
    assert_eq!(resp.status(), http::StatusCode::SEE_OTHER);
    assert_eq!(
        resp.headers().get(http::header::LOCATION).unwrap(),
        "/login?next=%2Fs%2Fdemo"
    );

    // API and WebSocket paths fail closed instead of redirecting an XHR/WS.
    let resp = client.get(format!("{base}/api/files?path=")).send().await?;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);

    let resp = client
        .get(format!("{base}/api/files?path="))
        .header(
            http::header::COOKIE,
            "sshx_board_auth=v1:9999999999:tampered",
        )
        .send()
        .await?;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);

    let resp = client.get(format!("{base}/api/s/demo")).send().await?;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);

    // Wrong passwords do not mint a cookie.
    let resp = client
        .post(format!("{base}/login"))
        .form(&[("password", "wrong"), ("next", "/go")])
        .send()
        .await?;
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
    assert!(resp.headers().get(http::header::SET_COOKIE).is_none());

    // Correct passwords mint a signed, long-lived, browser-safe auth cookie.
    let resp = client
        .post(format!("{base}/login"))
        .form(&[("password", "test board password"), ("next", "/s/demo#key")])
        .send()
        .await?;
    assert_eq!(resp.status(), http::StatusCode::SEE_OTHER);
    assert_eq!(
        resp.headers().get(http::header::LOCATION).unwrap(),
        "/s/demo#key"
    );
    let set_cookie = resp
        .headers()
        .get(http::header::SET_COOKIE)
        .unwrap()
        .to_str()?;
    assert!(set_cookie.starts_with("sshx_board_auth=v1:"));
    assert!(set_cookie.contains("Max-Age=2592000"));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("Secure"));
    assert!(set_cookie.contains("SameSite=Lax"));
    let cookie = auth_cookie_from_response(&resp);

    let resp = client
        .get(format!("{base}/api/files?path="))
        .header(http::header::COOKIE, &cookie)
        .send()
        .await?;
    assert_eq!(resp.status(), http::StatusCode::OK);

    let file_name = format!("sshx_auth_test_{}.txt", server.local_addr().port());
    let file_path = std::path::PathBuf::from(format!("/root/maw-workspace/{file_name}"));
    tokio::fs::write(&file_path, b"authed file").await?;
    let resp = client
        .get(format!("{base}/api/file?path={file_name}"))
        .header(http::header::COOKIE, &cookie)
        .send()
        .await?;
    assert_eq!(resp.status(), http::StatusCode::OK);
    assert!(resp.text().await?.contains("\"content\":\"authed file\""));
    tokio::fs::remove_file(&file_path).await.ok();

    Ok(())
}

#[tokio::test]
async fn test_healthz() -> Result<()> {
    let mut options = ServerOptions::default();
    options.board_password = Some("test board password".to_string());
    let server = TestServer::new_with_options(options).await;
    let client = reqwest::Client::new();
    let base = format!("http://{}", server.local_addr());

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
