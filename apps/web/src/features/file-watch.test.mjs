import { test } from "node:test";
import assert from "node:assert/strict";
import {
  announceFilesChanged,
  createFileWatch,
  directoriesToReload,
  onFilesChanged,
  touchesFile,
} from "./file-watch.ts";

class FakeSocket {
  onopen = null;
  onmessage = null;
  onerror = null;
  onclose = null;
  closes = 0;
  constructor(url) {
    this.url = url;
  }
  message(value) {
    this.onmessage?.({ data: JSON.stringify(value) });
  }
  remoteClose() {
    this.onclose?.({});
  }
  close() {
    this.closes++;
  }
}
function fakeClock() {
  let id = 0;
  const pending = new Map();
  return {
    pending,
    setTimer(callback, delay) {
      const key = ++id;
      pending.set(key, { callback, delay });
      return key;
    },
    clearTimer(key) {
      pending.delete(key);
    },
    fire() {
      for (const [key, timer] of [...pending]) {
        pending.delete(key);
        timer.callback();
      }
    },
  };
}
function harness() {
  const sockets = [],
    changes = [],
    clock = fakeClock();
  const watch = createFileWatch({
    createSocket: url => {
      const socket = new FakeSocket(url);
      sockets.push(socket);
      return socket;
    },
    url: id => `ws://host/${id}`,
    onChange: (id, paths, git) => changes.push({ id, paths, git }),
    setTimer: clock.setTimer,
    clearTimer: clock.clearTimer,
  });
  return { watch, sockets, changes, clock };
}

test("a change reloads the loaded folders it touches, and only those", () => {
  assert.deepEqual(directoriesToReload(["src/app.ts", "src/lib", "docs/a.md"], ["", "src", "src/lib"]), [
    "src",
    "src/lib",
  ]);
  assert.deepEqual(directoriesToReload(["README.md"], ["", "src"]), [""]);
  assert.deepEqual(directoriesToReload(["deep/unopened/file.ts"], ["", "src"]), []);
  assert.equal(directoriesToReload(null, [""]), null);
});

test("an open file follows changes to its own path, or to anything", () => {
  assert.equal(touchesFile(["src/app.ts"], "src/app.ts"), true);
  assert.equal(touchesFile(["src/app.tsx"], "src/app.ts"), false);
  assert.equal(touchesFile(null, "src/app.ts"), true);
});

test("listeners hear every workspace until they unsubscribe", () => {
  const heard = [];
  const stop = onFilesChanged((id, paths) => heard.push([id, paths]));
  announceFilesChanged("w1", ["a"]);
  stop();
  announceFilesChanged("w1", ["b"]);
  assert.deepEqual(heard, [["w1", ["a"]]]);
});

test("sync opens one socket per workspace and closes the ones no longer shown", () => {
  const { watch, sockets } = harness();
  watch.sync(["w1", "w2"]);
  watch.sync(["w2", "w1"]);
  assert.deepEqual(
    sockets.map(socket => socket.url),
    ["ws://host/w1", "ws://host/w2"],
  );
  watch.sync(["w2"]);
  assert.equal(sockets[0].closes, 1);
  assert.equal(sockets[1].closes, 0);
  watch.dispose();
  assert.equal(sockets[1].closes, 1);
});

test("changes are passed on, and a reconnect reports everything as changed", () => {
  const { watch, sockets, changes, clock } = harness();
  watch.sync(["w1"]);
  sockets[0].message({ type: "ready" });
  sockets[0].message({ type: "changed", paths: ["a.ts"], git: false });
  sockets[0].message({ type: "changed", paths: null, git: true });
  assert.deepEqual(changes, [
    { id: "w1", paths: ["a.ts"], git: false },
    { id: "w1", paths: null, git: true },
  ]);
  sockets[0].remoteClose();
  assert.equal([...clock.pending.values()][0].delay, 1000);
  clock.fire();
  assert.equal(sockets.length, 2);
  sockets[1].message({ type: "ready" });
  assert.deepEqual(changes.at(-1), { id: "w1", paths: null, git: true });
});

test("reconnects back off, and waking the tab retries at once", () => {
  const { watch, sockets, clock } = harness();
  watch.sync(["w1"]);
  sockets[0].remoteClose();
  clock.fire();
  sockets[1].remoteClose();
  assert.equal([...clock.pending.values()][0].delay, 2000);
  watch.wake();
  assert.equal(clock.pending.size, 0);
  assert.equal(sockets.length, 3);
});

test("a stale wake replaces a socket that only looks open, and the new one resyncs everything", () => {
  const { watch, sockets, changes } = harness();
  watch.sync(["w1"]);
  sockets[0].message({ type: "ready" });
  watch.wake();
  assert.equal(sockets.length, 1, "a brief return trusts an open socket");
  watch.wake(true);
  assert.equal(sockets.length, 2);
  assert.equal(sockets[0].closes, 1);
  sockets[0].message({ type: "changed", paths: ["late.md"] });
  sockets[1].message({ type: "ready" });
  assert.deepEqual(
    changes,
    [{ id: "w1", paths: null, git: true }],
    "the retired socket is silent; the new one reports a full resync",
  );
});

test("a host that cannot watch is not asked again", () => {
  const { watch, sockets, clock } = harness();
  watch.sync(["w1"]);
  sockets[0].message({ type: "unavailable", message: "inotify limit" });
  sockets[0].remoteClose();
  assert.equal(clock.pending.size, 0);
  watch.sync([]);
  watch.sync(["w1"]);
  assert.equal(sockets.length, 1);
});
