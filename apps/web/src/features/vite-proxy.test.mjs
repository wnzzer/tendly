import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer as createHttpServer } from "node:http";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { createServer as createViteServer } from "vite";

function deadline(operation, milliseconds, message) {
  let timer;
  return Promise.race([
    operation,
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    }),
  ]).finally(() => clearTimeout(timer));
}

function opens(socket, milliseconds) {
  return new Promise((resolve, reject) => {
    const finish = error => {
      clearTimeout(timer);
      socket.removeEventListener("open", onOpen);
      socket.removeEventListener("error", onError);
      socket.removeEventListener("close", onClose);
      if (error) reject(error);
      else resolve();
    };
    const onOpen = () => finish();
    const onError = () => finish(new Error("Fixture WebSocket proxy handshake failed"));
    const onClose = () => finish(new Error("Fixture WebSocket closed before opening"));
    const timer = setTimeout(() => finish(new Error("Fixture WebSocket proxy handshake timed out")), milliseconds);
    socket.addEventListener("open", onOpen);
    socket.addEventListener("error", onError);
    socket.addEventListener("close", onClose);
  });
}

test(
  "the real Vite configuration forwards a native WebSocket handshake and preserves the session path",
  { timeout: 15_000 },
  async () => {
    const upstreamSockets = new Set(),
      proxySockets = new Set(),
      requests = [];
    let upstreamBytes = 0,
      downstreamMessages = 0,
      vite,
      client;
    const upstream = createHttpServer((_request, response) => {
      response.writeHead(404);
      response.end();
    });
    upstream.on("connection", socket => {
      upstreamSockets.add(socket);
      socket.on("close", () => upstreamSockets.delete(socket));
      socket.on("error", () => {});
    });
    upstream.on("upgrade", (request, socket, head) => {
      requests.push({
        path: request.url,
        upgrade: request.headers.upgrade,
        version: request.headers["sec-websocket-version"],
        headBytes: head.length,
      });
      const key = request.headers["sec-websocket-key"];
      if (typeof key !== "string") {
        socket.destroy();
        return;
      }
      const accept = createHash("sha1")
        .update(key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11")
        .digest("base64");
      // A protocol handshake only: no application frames, native output, prompts or terminal process.
      socket.write(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: " +
          accept +
          "\r\n\r\n",
      );
      socket.on("data", chunk => {
        upstreamBytes += chunk.length;
      });
    });

    try {
      const listening = once(upstream, "listening");
      upstream.listen(0, "127.0.0.1");
      await deadline(listening, 2_000, "Fixture upstream did not listen");
      const upstreamAddress = upstream.address();
      assert.ok(upstreamAddress && typeof upstreamAddress === "object");
      const target = `http://127.0.0.1:${upstreamAddress.port}`;

      const creating = createViteServer({
        root: fileURLToPath(new URL("../../", import.meta.url)),
        configFile: fileURLToPath(new URL("../../vite.config.ts", import.meta.url)),
        appType: "custom",
        logLevel: "silent",
        optimizeDeps: { noDiscovery: true },
        server: {
          host: "127.0.0.1",
          port: 0,
          strictPort: true,
          hmr: false,
          ws: false,
          // Override only the destination; ws:true must come from the real application configuration.
          proxy: { "/api": { target } },
        },
      });
      try {
        vite = await deadline(creating, 4_000, "Fixture Vite configuration did not initialize");
      } catch (error) {
        void creating.then(server => server.close()).catch(() => {});
        throw error;
      }
      assert.equal(vite.config.server.proxy["/api"].target, target);
      assert.equal(vite.config.server.proxy["/api"].ws, true);
      assert.equal(vite.config.server.ws, false); // Disables Vite/HMR WS, not proxy upgrade handling.
      assert.ok(vite.httpServer);
      vite.httpServer.on("connection", socket => {
        proxySockets.add(socket);
        socket.on("close", () => proxySockets.delete(socket));
        socket.on("error", () => {});
      });
      await deadline(vite.listen(), 2_000, "Fixture Vite server did not listen");
      const address = vite.httpServer.address();
      assert.ok(address && typeof address === "object");
      const path = "/api/sessions/fixture/pty/ws";
      client = new WebSocket(`ws://127.0.0.1:${address.port}${path}`);
      client.addEventListener("message", () => {
        downstreamMessages++;
      });
      await opens(client, 3_000);
      assert.equal(client.readyState, WebSocket.OPEN);
      assert.deepEqual(requests, [{ path, upgrade: "websocket", version: "13", headBytes: 0 }]);
      assert.equal(upstreamBytes, 0); // No send(), resize, input, login or application traffic.
      assert.equal(downstreamMessages, 0);
    } finally {
      try {
        client?.close();
      } catch {
        /* Also clean up a handshake that failed before open. */
      }
      for (const socket of proxySockets) socket.destroy();
      for (const socket of upstreamSockets) socket.destroy();
      const closeUpstream = new Promise((resolve, reject) =>
        upstream.close(error => (error && error.code !== "ERR_SERVER_NOT_RUNNING" ? reject(error) : resolve())),
      );
      await deadline(Promise.all([vite?.close(), closeUpstream]), 3_000, "Fixture proxy cleanup timed out");
    }
    assert.equal(upstream.listening, false);
    assert.equal(vite?.httpServer?.listening ?? false, false);
  },
);
