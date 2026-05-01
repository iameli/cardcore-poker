/**
 * WebSocket room client
 * Connects to the cardcore room server for multiplayer coordination
 */

export class RoomClient {
  constructor() {
    this.ws = null;
    this.handlers = new Map();
    this.reconnectTimer = null;
    this.reconnectAttempts = 0;
    this.maxReconnectAttempts = 20;
    this.playerId = null;
    this.roomId = null;
    this.playerName = null;
    this.playerDid = null;
    this.connected = false;
    this.destroyed = false;
    this.spectating = false;
    // Route through Vite proxy in dev (avoids mixed-content on HTTPS)
    // In production, point directly at the server
    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    this.url = `${proto}//${window.location.host}/ws`;
    this._pending = [];
  }

  connect(playerId, roomId, playerName, did) {
    this.spectating = false;
    this._doConnect(playerId, roomId, playerName, did, "join");
  }

  spectate(playerId, roomId, playerName, did) {
    this.spectating = true;
    this._doConnect(playerId, roomId, playerName, did, "spectate");
  }

  _doConnect(playerId, roomId, playerName, did, joinType) {
    this.destroyed = false;

    // Null out old WebSocket handlers before closing to prevent
    // stale onclose from spawning a ghost reconnect
    if (this.ws) {
      this.ws.onopen = null;
      this.ws.onclose = null;
      this.ws.onerror = null;
      this.ws.onmessage = null;
      try {
        this.ws.close();
      } catch {
        /* ignore */
      }
      this.ws = null;
    }
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    this.playerId = playerId;
    this.roomId = roomId;
    this.playerName = playerName;
    this.playerDid = did || null;
    this._pending = [];
    this.reconnectAttempts = 0;

    try {
      this.ws = new WebSocket(this.url);
    } catch (err) {
      console.error("WebSocket construction failed:", err);
      this.connected = false;
      this.emit("error", { message: "Could not connect to server" });
      return;
    }

    this.ws.onopen = () => {
      this.connected = true;
      this.reconnectAttempts = 0;
      console.log(`[WS] Connection opened, sending ${joinType}...`);
      // Always send join/spectate first
      this._sendRaw({
        type: joinType,
        playerId,
        roomId,
        playerName,
        did: did || null,
      });
      // Flush any messages queued before connection
      const pendingCount = this._pending.length;
      for (const msg of this._pending) {
        console.log("[WS] Flushing queued message:", msg.type);
        this._sendRaw(msg);
      }
      this._pending = [];
      console.log(`[WS] Flushed ${pendingCount} queued messages`);
      this.emit("connected", {});
    };

    this.ws.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data);
        console.log("[WS] Received:", msg.type, JSON.stringify(msg).slice(0, 300));
        this.emit(msg.type, msg);
      } catch (e) {
        console.log("[WS] Malformed message:", event.data);
      }
    };

    this.ws.onclose = () => {
      this.connected = false;
      this.emit("disconnected", {});

      if (this.destroyed) return;

      // Exponential backoff with max attempts
      if (this.reconnectAttempts < this.maxReconnectAttempts) {
        const delay = Math.min(1000 * Math.pow(2, this.reconnectAttempts), 30000);
        this.reconnectAttempts++;
        this.reconnectTimer = setTimeout(() => {
          if (!this.destroyed && this.playerId && this.roomId) {
            if (this.spectating) {
              this.spectate(this.playerId, this.roomId, this.playerName, this.playerDid);
            } else {
              this.connect(this.playerId, this.roomId, this.playerName, this.playerDid);
            }
          }
        }, delay);
      } else {
        this.emit("error", {
          message: "Max reconnect attempts reached. Is the server running?",
        });
      }
    };

    this.ws.onerror = () => {
      // onclose will fire next, handle reconnect there
    };
  }

  _sendRaw(msg) {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      const json = JSON.stringify(msg);
      console.log("[WS] Sending:", msg.type, json.slice(0, 200));
      this.ws.send(json);
      return true;
    }
    console.log("[WS] Cannot send (not open), buffering:", msg.type);
    return false;
  }

  send(msg) {
    if (this._sendRaw(msg)) return;
    console.log("[WS] Buffering message:", msg.type);
    this._pending.push(msg);
  }

  sit(seat) {
    this.send({
      type: "sit",
      seat,
      playerId: this.playerId,
      roomId: this.roomId,
    });
  }

  ready() {
    this.send({ type: "ready", playerId: this.playerId, roomId: this.roomId });
  }

  sendAction(action) {
    this.send({
      type: "action",
      playerId: this.playerId,
      roomId: this.roomId,
      action,
    });
  }

  leave() {
    this.destroyed = true;
    if (this.ws) {
      this.ws.onclose = null; // prevent reconnect
      try {
        this.send({
          type: "leave",
          playerId: this.playerId,
          roomId: this.roomId,
        });
        this.ws.close();
      } catch {
        /* ignore */
      }
      this.ws = null;
    }
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  on(event, handler) {
    if (!this.handlers.has(event)) {
      this.handlers.set(event, []);
    }
    this.handlers.get(event).push(handler);
  }

  off(event, handler) {
    if (!this.handlers.has(event)) return;
    const handlers = this.handlers.get(event);
    const idx = handlers.indexOf(handler);
    if (idx !== -1) handlers.splice(idx, 1);
  }

  emit(event, data) {
    const handlers = this.handlers.get(event) || [];
    for (const handler of handlers) {
      try {
        handler(data);
      } catch (e) {
        console.error(`Error in handler for ${event}:`, e);
      }
    }
  }

  destroy() {
    this.destroyed = true;
    this.leave();
    this.handlers.clear();
  }
}

// Singleton
export const roomClient = new RoomClient();
