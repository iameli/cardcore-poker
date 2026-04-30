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
        } else {
          const autoSeat = room.players.length;
          room.players.push({
            id: playerId,
            name: playerName,
            did: playerDid,
            ws,
            seat: autoSeat,
          });
          console.log(
            `[JOIN] ${playerName} (${playerId}) joined room ${roomId} at auto-seat ${autoSeat}. Total players: ${room.players.length}`,
          );
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

        // Check if all players are ready
        if (room.players.length >= 2 && room.players.every((p) => p.ready)) {
          console.log(
            `[GAME] All players ready in room ${roomId}, starting game!`,
          );
          room.gameActive = true;
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

        // Send current room state to the spectator
        ws.send(JSON.stringify({
          type: "room_update",
          room: sanitizeRoom(room),
        }));

        // If a game is already in progress, send game_start so spectator can sync
        if (room.gameActive && room.gamePlayers) {
          console.log(
            `[SPECTATE] Game in progress in room ${roomId}, sending game_start to spectator ${playerName}`,
          );
          ws.send(JSON.stringify({
            type: "game_start",
            players: room.gamePlayers,
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
  // Quiet no-op: WebSocket opened but join never processed (HMR, rapid reconnect)
  if (!rid || !rooms.has(rid)) return;
  const room = rooms.get(rid);

  // Check if this is a spectator leaving
  if (ws.isSpectator) {
    const leavingSpectator = room.spectators.find((s) => s.id === pid);
    if (!leavingSpectator) return;
    console.log(`[LEAVE] Spectator ${leavingSpectator?.name || 'Unknown'} (${pid}) left room ${rid}`);
    room.spectators = room.spectators.filter((s) => s.id !== pid);
    if (room.players.length === 0 && room.spectators.length === 0) {
      rooms.delete(rid);
      console.log(`[LEAVE] Room ${rid} deleted (no players or spectators left)`);
    }
    return;
  }

  const leavingPlayer = room.players.find((p) => p.id === pid);
  if (!leavingPlayer) return;
  console.log(`[LEAVE] ${leavingPlayer?.name || 'Unknown'} (${pid}) removed from room ${rid}`);
  room.players = room.players.filter((p) => p.id !== pid);
  if (room.players.length === 0 && room.spectators.length === 0) {
    rooms.delete(rid);
    console.log(`[LEAVE] Room ${rid} deleted (no players or spectators left)`);
  } else {
    broadcastRoom(rid, {
      type: "room_update",
      room: sanitizeRoom(room),
    });
  }
}

function broadcastRoom(roomId, msg, excludePlayerId = null) {
  const room = rooms.get(roomId);
  if (!room) {
    console.log(`[BROADCAST] Room ${roomId} not found!`);
    return;
  }
  const data = JSON.stringify(msg);
  let sentCount = 0;
  for (const player of room.players) {
    if (player.id !== excludePlayerId && player.ws.readyState === 1) {
      player.ws.send(data);
      sentCount++;
    }
  }
  // Also send to the sender for confirmation
  if (excludePlayerId) {
    const sender = room.players.find((p) => p.id === excludePlayerId);
    if (sender && sender.ws.readyState === 1) {
      sender.ws.send(data);
      sentCount++;
    }
  }
  // Broadcast to spectators too
  for (const spectator of room.spectators) {
    if (spectator.ws.readyState === 1) {
      spectator.ws.send(data);
      sentCount++;
    }
  }
  console.log(`[BROADCAST] Sent ${msg.type} to ${sentCount} clients in room ${roomId}`);
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

// API endpoint for room list
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

// Create room endpoint
app.post("/api/rooms", express.json(), (req, res) => {
  const roomId = uuidv4().slice(0, 8);
  rooms.set(roomId, {
    id: roomId,
    players: [],
    spectators: [],
    gameState: null,
    createdAt: Date.now(),
  });
  res.json({ roomId });
});

// Store AT Protocol URI for a room
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
