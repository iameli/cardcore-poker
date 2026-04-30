import { WebSocketServer } from "ws";
import { v4 as uuidv4 } from "uuid";
import express from "express";
import http from "http";

const PORT = process.env.PORT || 3003;
const rooms = new Map();

const app = express();
const server = http.createServer(app);
const wss = new WebSocketServer({ server });

console.log(`Cardcore room server listening on ws://localhost:${PORT}`);

wss.on("connection", (ws) => {
  let playerId = null;
  let roomId = null;
  let playerName = null;

  ws.on("message", (raw) => {
    let msg;
    try {
      msg = JSON.parse(raw.toString());
    } catch {
      return;
    }

    switch (msg.type) {
      case "join": {
        playerId = msg.playerId;
        roomId = msg.roomId;
        playerName = msg.playerName;
        const playerDid = msg.did || null;

        if (!rooms.has(roomId)) {
          rooms.set(roomId, {
            id: roomId,
            players: [],
            spectators: [],
            gameState: null,
            actionHistory: [],
            createdAt: Date.now(),
          });
          console.log(`[ROOM] Created room ${roomId}`);
        }

        const room = rooms.get(roomId);
        const existing = room.players.find((p) => p.id === playerId);
        if (existing) {
          existing.ws = ws;
          existing.name = playerName;
          if (playerDid) existing.did = playerDid;
          console.log(
            `[JOIN] ${playerName} (${playerId}) reconnected to room ${roomId}`,
          );
          // Send game state sync if game is active
          if (room.gameActive && room.actionHistory.length > 0) {
            console.log(`[SYNC] Sending game state to reconnected ${playerName}`);
            ws.send(JSON.stringify({
              type: "game_state_sync",
              players: room.gamePlayers,
              history: room.actionHistory,
            }));
          }
        } else {
          if (room.gameActive) {
            if (!room.pendingPlayers) room.pendingPlayers = [];
            room.pendingPlayers.push({ id: playerId, name: playerName, did: playerDid, ws });
            ws.isPending = true;
            console.log("[JOIN] " + playerName + " (" + playerId + ") queued for next hand in room " + roomId);
            ws.send(JSON.stringify({ type: "game_state_sync", players: room.gamePlayers, history: room.actionHistory }));
          } else {
            const autoSeat = room.players.length;
            room.players.push({ id: playerId, name: playerName, did: playerDid, ws, seat: autoSeat });
            console.log("[JOIN] " + playerName + " (" + playerId + ") joined room " + roomId + " at auto-seat " + autoSeat + ". Total players: " + room.players.length);
          }
        }

        ws.roomId = roomId;
        ws.playerId = playerId;

        broadcastRoom(roomId, {
          type: "room_update",
          room: sanitizeRoom(room),
        });
        break;
      }

      case "sit": {
        const room = rooms.get(roomId);
        if (!room) {
          console.log(`[SIT] Room ${roomId} not found!`);
          return;
        }
        const player = room.players.find((p) => p.id === playerId);
        if (!player) {
          console.log(`[SIT] Player ${playerId} not found in room ${roomId}!`);
          return;
        }
        console.log(`[SIT] ${playerName} (${playerId}) moving from seat ${player.seat} to seat ${msg.seat} in room ${roomId}`);
        player.seat = msg.seat;
        broadcastRoom(roomId, {
          type: "room_update",
          room: sanitizeRoom(room),
        });
        break;
      }

      case "ready": {
        const room = rooms.get(roomId);
        if (!room) return;
        const player = room.players.find((p) => p.id === playerId);
        if (!player) return;
        player.ready = true;
        console.log(
          `[READY] ${playerName} (${playerId}) is ready in room ${roomId}`,
        );

        broadcastRoom(roomId, {
          type: "room_update",
          room: sanitizeRoom(room),
        });

        if (room.players.length >= 2 && room.players.every((p) => p.ready)) {
          console.log(
            `[GAME] All players ready in room ${roomId}, starting game!`,
          );
          room.gameActive = true;
          room.actionHistory = [];
          if (room.pendingPlayers && room.pendingPlayers.length > 0) {
            for (const pp of room.pendingPlayers) {
              pp.seat = room.players.length;
              room.players.push(pp);
            }
            room.pendingPlayers = [];
          }
          room.gamePlayers = room.players.map((p) => ({
            id: p.id,
            name: p.name,
            did: p.did || null,
            seat: p.seat,
          }));
          broadcastRoom(roomId, {
            type: "game_start",
            players: room.gamePlayers,
          });
        }
        break;
      }

      case "action": {
        console.log(
          `[ACTION] ${playerName} (${playerId}) action: ${msg.action?.type || JSON.stringify(msg.action)}`,
        );
        const room = rooms.get(roomId);
        if (room && msg.action && (msg.action.type === "wasm_table" || msg.action.type === "wasm_action")) {
          room.actionHistory.push({ playerId, action: msg.action });
        }
        broadcastRoom(
          roomId,
          {
            type: "game_action",
            playerId,
            action: msg.action,
          },
          playerId,
        );
        break;
      }

      case "spectate": {
        playerId = msg.playerId;
        roomId = msg.roomId;
        playerName = msg.playerName;

        if (!rooms.has(roomId)) {
          rooms.set(roomId, {
            id: roomId,
            players: [],
            spectators: [],
            gameState: null,
            actionHistory: [],
            createdAt: Date.now(),
          });
          console.log(`[ROOM] Created room ${roomId} (via spectate)`);
        }

        const room = rooms.get(roomId);
        const existing = room.spectators.find((s) => s.id === playerId);
        if (existing) {
          existing.ws = ws;
          existing.name = playerName;
          console.log(
            `[SPECTATE] ${playerName} (${playerId}) reconnected to room ${roomId} as spectator`,
          );
        } else {
          room.spectators.push({
            id: playerId,
            name: playerName,
            ws,
          });
          console.log(
            `[SPECTATE] ${playerName} (${playerId}) now spectating room ${roomId}. Total spectators: ${room.spectators.length}`,
          );
        }

        ws.roomId = roomId;
        ws.playerId = playerId;
        ws.isSpectator = true;

        ws.send(JSON.stringify({
          type: "room_update",
          room: sanitizeRoom(room),
        }));

        if (room.gameActive && room.gamePlayers) {
          console.log(
            `[SPECTATE] Game in progress, sending game_state_sync to ${playerName}`,
          );
          ws.send(JSON.stringify({
            type: "game_state_sync",
            players: room.gamePlayers,
            history: room.actionHistory || [],
          }));
        }
        break;
      }

      case "leave": {
        console.log(
          `[LEAVE] ${playerName} (${playerId}) leaving room ${roomId}`,
        );
        leaveRoom(ws);
        break;
      }
    }
  });

  ws.on("close", () => {
    leaveRoom(ws);
  });

  ws.on("error", () => {
    leaveRoom(ws);
  });
});

function leaveRoom(ws) {
  const rid = ws.roomId;
  const pid = ws.playerId;
  if (!rid || !rooms.has(rid)) return;
  const room = rooms.get(rid);

  if (ws.isPending) {
    if (room.pendingPlayers) room.pendingPlayers = room.pendingPlayers.filter(p => p.id !== pid);
    return;
  }
  if (ws.isSpectator) {
    const leavingSpectator = room.spectators.find((s) => s.id === pid);
    if (!leavingSpectator) return;
    console.log(`[LEAVE] Spectator ${leavingSpectator?.name || 'Unknown'} (${pid}) left room ${rid}`);
    room.spectators = room.spectators.filter((s) => s.id !== pid);
    if (room.players.length === 0 && room.spectators.length === 0) {
      rooms.delete(rid);
    }
    return;
  }

  const leavingPlayer = room.players.find((p) => p.id === pid);
  if (!leavingPlayer) {
    if (room.pendingPlayers) room.pendingPlayers = room.pendingPlayers.filter(p => p.id !== pid);
    return;
  }
  console.log('[LEAVE] ' + (leavingPlayer?.name || 'Unknown') + ' (' + pid + ') removed from room ' + rid);
  if (room.gameActive) {
    broadcastRoom(rid, { type: 'game_action', playerId: pid, action: { type: 'wasm_action', cbor: 'omUkdHlwZXgYcmUuY2FyZGNvLnBva2VyLmRlZnMjYmV0ZmFjdGlvbmRmb2xk' } }, pid);
  }
  room.players = room.players.filter((p) => p.id !== pid);
  if (room.players.length === 0 && room.spectators.length === 0) {
    rooms.delete(rid);
  } else {
    broadcastRoom(rid, {
      type: "room_update",
      room: sanitizeRoom(room),
    });
  }
}

function broadcastRoom(roomId, msg, excludePlayerId = null) {
  const room = rooms.get(roomId);
  if (!room) return;
  const data = JSON.stringify(msg);
  let sentCount = 0;
  for (const player of room.players) {
    if (player.id !== excludePlayerId && player.ws.readyState === 1) {
      player.ws.send(data);
      sentCount++;
    }
  }
  if (excludePlayerId) {
    const sender = room.players.find((p) => p.id === excludePlayerId);
    if (sender && sender.ws.readyState === 1) {
      sender.ws.send(data);
      sentCount++;
    }
  }
  for (const spectator of room.spectators) {
    if (spectator.ws.readyState === 1) {
      spectator.ws.send(data);
      sentCount++;
    }
  }
}

function sanitizeRoom(room) {
  return {
    id: room.id,
    players: room.players.map((p) => ({
      id: p.id,
      name: p.name,
      did: p.did || null,
      seat: p.seat,
      ready: p.ready || false,
    })),
    spectatorCount: (room.spectators || []).length,
    createdAt: room.createdAt,
  };
}

app.get("/api/rooms", (req, res) => {
  const roomList = [];
  for (const [id, room] of rooms) {
    roomList.push({
      id,
      playerCount: room.players.length,
      spectatorCount: (room.spectators || []).length,
      hasGame: room.gameActive || false,
      createdAt: room.createdAt,
    });
  }
  res.json(roomList);
});

app.post("/api/rooms", express.json(), (req, res) => {
  const roomId = uuidv4().slice(0, 8);
  rooms.set(roomId, {
    id: roomId,
    players: [],
    spectators: [],
    gameState: null,
    actionHistory: [],
    createdAt: Date.now(),
  });
  res.json({ roomId });
});

app.put("/api/rooms/:roomId/atp", express.json(), (req, res) => {
  const { roomId } = req.params;
  const room = rooms.get(roomId);
  if (!room) {
    return res.status(404).json({ error: 'Room not found' });
  }
  room.atpUri = req.body.atpUri;
  console.log(`[ATP] Room ${roomId} linked to AT Protocol: ${room.atpUri}`);
  res.json({ ok: true });
});

server.listen(PORT, () => {
  console.log(`Server running on http://localhost:${PORT}`);
});
