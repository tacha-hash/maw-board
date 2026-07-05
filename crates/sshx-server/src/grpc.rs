//! Defines gRPC routes and application request logic.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use base64::prelude::{Engine as _, BASE64_STANDARD};
use hmac::Mac;
use sshx_core::proto::{
    client_update::ClientMessage, server_update::ServerMessage, sshx_service_server::SshxService,
    ClientUpdate, CloseRequest, CloseResponse, JoinRequest, JoinResponse, OpenRequest,
    OpenResponse, ServerUpdate,
};
use sshx_core::{rand_alphanumeric, BackendId, Sid};
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use tokio::time::{self, MissedTickBehavior};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::{Request, Response, Status, Streaming};
use tracing::{error, info, warn};

use crate::session::{Metadata, Session};
use crate::ServerState;

/// Interval for synchronizing sequence numbers with the client.
pub const SYNC_INTERVAL: Duration = Duration::from_secs(5);

/// Interval for measuring client latency.
pub const PING_INTERVAL: Duration = Duration::from_secs(2);

/// Server that handles gRPC requests from the sshx command-line client.
#[derive(Clone)]
pub struct GrpcServer(Arc<ServerState>);

impl GrpcServer {
    /// Construct a new [`GrpcServer`] instance with associated state.
    pub fn new(state: Arc<ServerState>) -> Self {
        Self(state)
    }
}

type RR<T> = Result<Response<T>, Status>;

#[tonic::async_trait]
impl SshxService for GrpcServer {
    type ChannelStream = ReceiverStream<Result<ServerUpdate, Status>>;

    async fn open(&self, request: Request<OpenRequest>) -> RR<OpenResponse> {
        let request = request.into_inner();
        let origin = self.0.override_origin().unwrap_or(request.origin);
        if origin.is_empty() {
            return Err(Status::invalid_argument("origin is empty"));
        }
        let name = rand_alphanumeric(10);
        info!(%name, "creating new session");

        match self.0.lookup(&name) {
            Some(_) => return Err(Status::already_exists("generated duplicate ID")),
            None => {
                let metadata = Metadata {
                    encrypted_zeros: request.encrypted_zeros,
                    name: request.name,
                    write_password_hash: request.write_password_hash,
                };
                let session = Session::new(metadata);
                let backend_id = session.register_backend("primary".to_string());
                debug_assert_eq!(backend_id, BackendId::PRIMARY);
                self.0.insert(&name, Arc::new(session));
            }
        };
        let token = backend_token(self.0.mac(), &name, BackendId::PRIMARY);
        let join_token = join_token(self.0.mac(), &name);
        let url = format!("{origin}/s/{name}");
        Ok(Response::new(OpenResponse {
            name,
            token,
            url,
            join_token,
        }))
    }

    async fn join(&self, request: Request<JoinRequest>) -> RR<JoinResponse> {
        let request = request.into_inner();
        validate_join_token(self.0.mac(), &request.name, &request.join_token)?;

        let session = match self.0.lookup(&request.name) {
            Some(session) => session,
            None => return Err(Status::not_found("session not found")),
        };

        // This proves the joining backend has the SAME encryption key as the
        // session (not authorization — join_token already proved that). A
        // mismatch here means the joiner's shells would be encrypted with a
        // key none of the connected browsers can decrypt: silent garbage,
        // not a crash, so it must be checked explicitly rather than assumed.
        let zeros_match: bool = request
            .encrypted_zeros
            .ct_eq(session.metadata().encrypted_zeros.as_ref())
            .into();
        if !zeros_match {
            return Err(Status::invalid_argument(
                "encrypted_zeros does not match session (wrong encryption key)",
            ));
        }

        let backend_name = if request.backend_name.is_empty() {
            "unnamed".to_string()
        } else {
            request.backend_name
        };
        let backend_id = session.register_backend(backend_name);
        let token = backend_token(self.0.mac(), &request.name, backend_id);
        info!(name = %request.name, %backend_id, "backend joined session");
        Ok(Response::new(JoinResponse { backend_id: backend_id.0, token }))
    }

    async fn channel(&self, request: Request<Streaming<ClientUpdate>>) -> RR<Self::ChannelStream> {
        let mut stream = request.into_inner();
        let first_update = match stream.next().await {
            Some(result) => result?,
            None => return Err(Status::invalid_argument("missing first message")),
        };
        let (session_name, backend_id) = match first_update.client_message {
            Some(ClientMessage::Hello(hello)) => {
                let (name, token) = hello
                    .split_once(',')
                    .ok_or_else(|| Status::invalid_argument("missing name and token"))?;
                validate_backend_token(self.0.mac(), name, BackendId::PRIMARY, token)?;
                (name.to_string(), BackendId::PRIMARY)
            }
            Some(ClientMessage::HelloBackend(hello)) => {
                let backend_id = BackendId(hello.backend_id);
                validate_backend_token(self.0.mac(), &hello.name, backend_id, &hello.token)?;
                (hello.name, backend_id)
            }
            _ => return Err(Status::invalid_argument("invalid first message")),
        };
        let session = match self.0.backend_connect(&session_name).await {
            Ok(Some(session)) => session,
            Ok(None) => return Err(Status::not_found("session not found")),
            Err(err) => {
                error!(?err, "failed to connect to backend session");
                return Err(Status::internal(err.to_string()));
            }
        };
        let backend_update_rx = session
            .connect_backend(backend_id)
            .map_err(|err| Status::already_exists(err.to_string()))?;

        // We now spawn an asynchronous task that sends updates to the client. Note that
        // when this task finishes, the sender end is dropped, so the receiver is
        // automatically closed.
        let (tx, rx) = mpsc::channel(16);
        tokio::spawn(async move {
            if let Err(err) =
                handle_streaming(&tx, &session, backend_id, &backend_update_rx, stream).await
            {
                warn!(?err, %backend_id, "connection exiting early due to an error");
            }
            session.disconnect_backend(backend_id);
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn close(&self, request: Request<CloseRequest>) -> RR<CloseResponse> {
        let request = request.into_inner();
        // Only the PRIMARY backend's token authorizes closing the whole
        // session — this falls out naturally since `validate_backend_token`
        // with BackendId::PRIMARY uses the original, un-suffixed derivation,
        // which a joined backend's token (suffixed with "backend"+id) will
        // never match. A joined backend disconnecting just calls
        // `disconnect_backend` (see `channel()`), never this RPC.
        validate_backend_token(self.0.mac(), &request.name, BackendId::PRIMARY, &request.token)?;
        info!("closing session {}", request.name);
        if let Err(err) = self.0.close_session(&request.name).await {
            error!(?err, "failed to close session {}", request.name);
            return Err(Status::internal(err.to_string()));
        }
        Ok(Response::new(CloseResponse {}))
    }
}

/// Compute the reconnect token for a specific backend. Backend 0 (the
/// primary, from `Open()`) always uses the original `mac(name)` derivation
/// unchanged, so old clients that only ever send `hello = "name,token"` keep
/// working exactly as before. Other backends get a domain-separated
/// derivation so their tokens can never collide with or be mistaken for the
/// primary's (see `close()`, which relies on this for its authorization).
fn backend_token(mac: impl Mac, name: &str, backend_id: BackendId) -> String {
    let mac = if backend_id == BackendId::PRIMARY {
        mac.chain_update(name)
    } else {
        mac.chain_update(name)
            .chain_update(b"backend")
            .chain_update(backend_id.0.to_le_bytes())
    };
    BASE64_STANDARD.encode(mac.finalize().into_bytes())
}

/// Compute the join_token for a session, returned once from `Open()` and
/// presented by any backend calling `Join()`. Uses the same server-wide MAC
/// secret as reconnect tokens, with a distinct domain-separation suffix, so
/// no additional per-session secret needs to be stored.
fn join_token(mac: impl Mac, name: &str) -> String {
    let mac = mac.chain_update(name).chain_update(b"join");
    BASE64_STANDARD.encode(mac.finalize().into_bytes())
}

/// Validate a presented reconnect token against the specific backend_id it
/// claims to be — not just "is this *a* valid token for *some* backend."
#[allow(clippy::result_large_err)]
fn validate_backend_token(
    mac: impl Mac,
    name: &str,
    backend_id: BackendId,
    token: &str,
) -> tonic::Result<()> {
    let Ok(token_bytes) = BASE64_STANDARD.decode(token) else {
        return Err(Status::unauthenticated("invalid token"));
    };
    let mac = if backend_id == BackendId::PRIMARY {
        mac.chain_update(name)
    } else {
        mac.chain_update(name)
            .chain_update(b"backend")
            .chain_update(backend_id.0.to_le_bytes())
    };
    mac.verify_slice(&token_bytes)
        .map_err(|_| Status::unauthenticated("invalid token"))
}

/// Validate a presented join_token for a session name.
#[allow(clippy::result_large_err)]
fn validate_join_token(mac: impl Mac, name: &str, token: &str) -> tonic::Result<()> {
    let Ok(token_bytes) = BASE64_STANDARD.decode(token) else {
        return Err(Status::unauthenticated("invalid join token"));
    };
    mac.chain_update(name)
        .chain_update(b"join")
        .verify_slice(&token_bytes)
        .map_err(|_| Status::unauthenticated("invalid join token"))
}

type ServerTx = mpsc::Sender<Result<ServerUpdate, Status>>;

/// Handle bidirectional streaming messages RPC messages.
async fn handle_streaming(
    tx: &ServerTx,
    session: &Session,
    backend_id: BackendId,
    backend_update_rx: &async_channel::Receiver<ServerMessage>,
    mut stream: Streaming<ClientUpdate>,
) -> Result<(), &'static str> {
    let mut sync_interval = time::interval(SYNC_INTERVAL);
    sync_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut ping_interval = time::interval(PING_INTERVAL);
    ping_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            // Send periodic sync messages to the client.
            _ = sync_interval.tick() => {
                let msg = ServerMessage::Sync(session.sequence_numbers());
                if !send_msg(tx, msg).await {
                    return Err("failed to send sync message");
                }
            }
            // Send periodic pings to the client.
            _ = ping_interval.tick() => {
                send_msg(tx, ServerMessage::Ping(get_time_ms())).await;
            }
            // Send buffered server updates to the client — from THIS backend's own channel.
            Ok(msg) = backend_update_rx.recv() => {
                if !send_msg(tx, msg).await {
                    return Err("failed to send update message");
                }
            }
            // Handle incoming client messages.
            maybe_update = stream.next() => {
                if let Some(Ok(update)) = maybe_update {
                    if !handle_update(tx, session, backend_id, update).await {
                        return Err("error responding to client update");
                    }
                } else {
                    // The client has hung up on their end.
                    return Ok(());
                }
            }
            // Exit on a session shutdown signal.
            _ = session.terminated() => {
                let msg = String::from("disconnecting because session is closed");
                send_msg(tx, ServerMessage::Error(msg)).await;
                return Ok(());
            }
        };
    }
}

/// Handles a singe update from the client. Returns `true` on success.
async fn handle_update(
    tx: &ServerTx,
    session: &Session,
    backend_id: BackendId,
    update: ClientUpdate,
) -> bool {
    session.access();
    match update.client_message {
        Some(ClientMessage::Hello(_)) | Some(ClientMessage::HelloBackend(_)) => {
            return send_err(tx, "unexpected hello".into()).await;
        }
        Some(ClientMessage::Data(data)) => {
            if let Err(err) = session.add_data(Sid(data.id), data.data, data.seq) {
                return send_err(tx, format!("add data: {:?}", err)).await;
            }
        }
        Some(ClientMessage::CreatedShell(new_shell)) => {
            let id = Sid(new_shell.id);
            let center = (new_shell.x, new_shell.y);
            if let Err(err) = session.add_shell(id, center, backend_id) {
                return send_err(tx, format!("add shell: {:?}", err)).await;
            }
        }
        Some(ClientMessage::ClosedShell(id)) => {
            if let Err(err) = session.close_shell(Sid(id)) {
                return send_err(tx, format!("close shell: {:?}", err)).await;
            }
        }
        Some(ClientMessage::Pong(ts)) => {
            let latency = get_time_ms().saturating_sub(ts);
            session.send_latency_measurement(latency);
        }
        Some(ClientMessage::Error(err)) => {
            // TODO: Propagate these errors to listeners on the web interface?
            error!(?err, "error received from client");
        }
        None => (), // Heartbeat message, ignored.
    }
    true
}

/// Attempt to send a server message to the client.
async fn send_msg(tx: &ServerTx, message: ServerMessage) -> bool {
    let update = Ok(ServerUpdate {
        server_message: Some(message),
    });
    tx.send(update).await.is_ok()
}

/// Attempt to send an error string to the client.
async fn send_err(tx: &ServerTx, err: String) -> bool {
    send_msg(tx, ServerMessage::Error(err)).await
}

fn get_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("system time is before the UNIX epoch")
        .as_millis() as u64
}
