/**
 * @file WebRTC peer-to-peer mesh for voice/video/screen-share.
 *
 * Each participant maintains one RTCPeerConnection per remote peer (full mesh).
 * Signaling (offer/answer/ICE) flows through the existing sshx WS relay via
 * Signal messages. Media tracks go peer-to-peer (or via TURN when NAT blocks).
 *
 * Usage flow:
 *   1. On join: create RtcMesh with local userId + send callback
 *   2. When a new user appears: mesh.addPeer(uid) — initiator sends offer
 *   3. When Signal arrives from relay: mesh.handleSignal(from, payload)
 *   4. To add local mic/video: mesh.addTrack(track)
 *   5. On leave: mesh.dispose()
 */

type SignalPayload =
  | { type: "offer"; sdp: string }
  | { type: "answer"; sdp: string }
  | { type: "ice"; candidate: RTCIceCandidateInit }
  | { type: "negotiate" };

export type OnTrackCallback = (
  uid: number,
  track: MediaStreamTrack,
  streams: readonly MediaStream[],
) => void;

export type SendSignal = (target: number, payload: string) => void;

export type RtcConfig = {
  iceServers: RTCIceServer[];
};

const DEFAULT_CONFIG: RtcConfig = {
  iceServers: [
    { urls: "stun:stun.l.google.com:19302" },
    {
      urls: "turn:openrelay.metered.ca:80",
      username: "openrelayproject",
      credential: "openrelayproject",
    },
    {
      urls: "turns:openrelay.metered.ca:443",
      username: "openrelayproject",
      credential: "openrelayproject",
    },
  ],
};

export class RtcMesh {
  readonly myUid: number;
  private peers = new Map<number, RTCPeerConnection>();
  private sendSignal: SendSignal;
  private onTrack: OnTrackCallback;
  private config: RtcConfig;
  private localTracks: MediaStreamTrack[] = [];
  private disposed = false;

  constructor(
    myUid: number,
    sendSignal: SendSignal,
    onTrack: OnTrackCallback,
    config?: Partial<RtcConfig>,
  ) {
    this.myUid = myUid;
    this.sendSignal = sendSignal;
    this.onTrack = onTrack;
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /** Add a peer and optionally initiate the connection (caller = higher UID). */
  addPeer(uid: number) {
    if (uid === this.myUid || this.peers.has(uid)) return;
    const pc = this.createPeerConnection(uid);
    this.peers.set(uid, pc);

    // Add any existing local tracks to the new peer connection.
    for (const track of this.localTracks) {
      pc.addTrack(track);
    }

    // Higher UID initiates the offer (deterministic tie-breaking).
    if (this.myUid > uid) {
      this.createOffer(uid, pc);
    }
  }

  /** Remove a peer (user left). */
  removePeer(uid: number) {
    const pc = this.peers.get(uid);
    if (pc) {
      pc.close();
      this.peers.delete(uid);
    }
  }

  /** Handle an incoming signaling message from the WS relay. */
  async handleSignal(from: number, payloadJson: string) {
    let payload: SignalPayload;
    try {
      payload = JSON.parse(payloadJson);
    } catch {
      return;
    }

    let pc = this.peers.get(from);
    if (!pc) {
      // Remote peer initiated before we knew about them — create lazily.
      pc = this.createPeerConnection(from);
      this.peers.set(from, pc);
      for (const track of this.localTracks) {
        pc.addTrack(track);
      }
    }

    if (payload.type === "offer") {
      await pc.setRemoteDescription({ type: "offer", sdp: payload.sdp });
      const answer = await pc.createAnswer();
      await pc.setLocalDescription(answer);
      this.sendSignal(
        from,
        JSON.stringify({ type: "answer", sdp: answer.sdp }),
      );
    } else if (payload.type === "answer") {
      await pc.setRemoteDescription({ type: "answer", sdp: payload.sdp });
    } else if (payload.type === "ice") {
      await pc.addIceCandidate(payload.candidate).catch(() => {});
    } else if (payload.type === "negotiate") {
      if (this.myUid > from) {
        await this.createOffer(from, pc);
      }
    }
  }

  /** Add a local media track (mic, camera, screen) to all current + future peers. */
  addTrack(track: MediaStreamTrack) {
    this.localTracks.push(track);
    for (const [uid, pc] of this.peers) {
      pc.addTrack(track);
      if (this.myUid > uid) {
        this.createOffer(uid, pc);
      } else {
        this.sendSignal(uid, JSON.stringify({ type: "negotiate" }));
      }
    }
  }

  /** Remove a local media track from all peers. */
  removeTrack(track: MediaStreamTrack) {
    this.localTracks = this.localTracks.filter((t) => t !== track);
    for (const [uid, pc] of this.peers) {
      const sender = pc.getSenders().find((s) => s.track === track);
      if (sender) pc.removeTrack(sender);
      if (this.myUid > uid) {
        this.createOffer(uid, pc);
      } else {
        this.sendSignal(uid, JSON.stringify({ type: "negotiate" }));
      }
    }
  }

  /** Tear down all peer connections. */
  dispose() {
    this.disposed = true;
    for (const [, pc] of this.peers) {
      pc.close();
    }
    this.peers.clear();
    this.localTracks = [];
  }

  private createPeerConnection(uid: number): RTCPeerConnection {
    const pc = new RTCPeerConnection({
      iceServers: this.config.iceServers,
    });

    pc.onicecandidate = (event) => {
      if (event.candidate && !this.disposed) {
        this.sendSignal(
          uid,
          JSON.stringify({ type: "ice", candidate: event.candidate.toJSON() }),
        );
      }
    };

    pc.ontrack = (event) => {
      if (!this.disposed) {
        this.onTrack(uid, event.track, event.streams);
      }
    };

    pc.oniceconnectionstatechange = () => {
      if (pc.iceConnectionState === "failed") {
        pc.restartIce();
      }
    };

    return pc;
  }

  private async createOffer(uid: number, pc: RTCPeerConnection) {
    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);
    this.sendSignal(
      uid,
      JSON.stringify({ type: "offer", sdp: offer.sdp }),
    );
  }
}
