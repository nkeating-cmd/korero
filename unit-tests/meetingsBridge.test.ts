// Kōrero (meetings reliability, 2026-09-25): ordering rules for the single
// route to meetings.json. Run with `bun test unit-tests` (or `bun run test:unit`).
//
// Lives outside `tests/` on purpose: that folder is Playwright's testDir, and
// Playwright would try to run a bun:test file as one of its own.

import { describe, expect, test } from "bun:test";
import {
  createMeetingsBridge,
  type MeetingsViewPort,
  type MeetingPatch,
} from "../src/stores/meetingsBridge";

type Doc = Record<string, unknown>;

/** In-memory meetings.json with an optional per-call delay, logging every op. */
function fakeStore(initial: Doc[], delayMs = 0) {
  const s = {
    doc: JSON.stringify(initial),
    failLoad: false,
    log: [] as string[],
  };
  const wait = () => new Promise((r) => setTimeout(r, delayMs));
  const io = {
    load: async () => {
      await wait();
      s.log.push("load");
      return s.failLoad
        ? { ok: false as const, error: "locked" }
        : { ok: true as const, data: s.doc };
    },
    save: async (json: string) => {
      await wait();
      s.log.push("save");
      s.doc = json;
      return { ok: true as const };
    },
  };
  const read = () => JSON.parse(s.doc) as Doc[];
  return { s, io, read };
}

/** A view port backed by a plain array, recording what it was handed. */
function fakeView(list: Doc[], activeId: string | null = null) {
  const got: string[] = [];
  const port: MeetingsViewPort = {
    applyPatch: (id: string, patch: MeetingPatch) => {
      const i = list.findIndex((m) => m.id === id);
      if (i < 0) return "missing";
      list[i] = { ...list[i], ...patch };
      got.push(`patch:${id}`);
      return id === activeId ? "shown" : "view";
    },
    addMeeting: (m, select) => {
      list.unshift(m);
      got.push(`add:${m.id}`);
      return select ? "shown" : "view";
    },
  };
  return { port, list, got };
}

const loadInto = async (
  bridge: ReturnType<typeof createMeetingsBridge>,
  list: Doc[],
) => {
  const r = await bridge.load();
  if (r.ok && r.data.trim()) list.push(...(JSON.parse(r.data) as Doc[]));
};

describe("meetingsBridge", () => {
  test("with no view, a patch is read-modify-written to disk", async () => {
    const { io, read } = fakeStore([{ id: "a", processed: "" }, { id: "b" }]);
    const bridge = createMeetingsBridge(io);
    expect(await bridge.deliverPatch("a", { processed: "notes" })).toBe("disk");
    expect(read()).toEqual([{ id: "a", processed: "notes" }, { id: "b" }]);
  });

  test("an unreadable store is never written (v1.29.0 R-02)", async () => {
    const { s, io } = fakeStore([{ id: "a" }]);
    s.failLoad = true;
    const bridge = createMeetingsBridge(io);
    expect(await bridge.deliverPatch("a", { processed: "x" })).toBe("failed");
    expect(await bridge.deliverNew({ id: "n" }, true)).toBe("failed");
    expect(s.log.filter((l) => l === "save")).toHaveLength(0);
  });

  test("a deleted meeting is reported missing and nothing is written", async () => {
    const { s, io } = fakeStore([{ id: "a" }]);
    const bridge = createMeetingsBridge(io);
    expect(await bridge.deliverPatch("gone", { processed: "x" })).toBe("missing");
    expect(s.log).toEqual(["load"]);
  });

  test("a ready view receives the patch and the disk is untouched", async () => {
    const { s, io } = fakeStore([{ id: "a" }]);
    const bridge = createMeetingsBridge(io);
    const v = fakeView([{ id: "a" }], "a");
    bridge.markReady(bridge.attach(v.port));
    expect(await bridge.deliverPatch("a", { processed: "x" })).toBe("shown");
    expect(v.list[0]).toEqual({ id: "a", processed: "x" });
    expect(s.log).toEqual([]);
  });

  test("a delivery during the view's load waits for the view, not the disk", async () => {
    const { s, io } = fakeStore([{ id: "a" }]);
    const bridge = createMeetingsBridge(io);
    const v = fakeView([], null);
    const token = bridge.attach(v.port);
    const pending = bridge.deliverPatch("a", { processed: "x" });
    await loadInto(bridge, v.list);
    bridge.markReady(token);
    expect(await pending).toBe("view");
    expect(v.list[0]).toEqual({ id: "a", processed: "x" });
    expect(s.log).toEqual(["load"]); // never written behind the view's back
  });

  test("THE v1.30.2 RACE: leave, work lands, come back — the new view sees it", async () => {
    // Slow IO so every step genuinely overlaps.
    const { io, read } = fakeStore([{ id: "a", you: "" }], 5);
    const bridge = createMeetingsBridge(io);
    const v1 = fakeView([], "a");
    const t1 = bridge.attach(v1.port);
    await loadInto(bridge, v1.list);
    bridge.markReady(t1);
    // The user leaves (nothing unsaved) while a transcription runs...
    bridge.detach(t1);
    // ...the transcription finishes with no view mounted...
    const landed = bridge.deliverPatch("a", { you: "the transcript" });
    // ...and the user comes back before that write has even finished.
    const v2 = fakeView([], "a");
    const t2 = bridge.attach(v2.port);
    await loadInto(bridge, v2.list); // queued BEHIND the patch write
    bridge.markReady(t2);
    expect(await landed).toBe("disk");
    // The new view loaded the patched document, so its autosave cannot revert it.
    expect(v2.list[0]).toEqual({ id: "a", you: "the transcript" });
    await bridge.save(JSON.stringify(v2.list)); // its next autosave
    expect(read()[0]).toEqual({ id: "a", you: "the transcript" });
  });

  test("a view's final save is written BEFORE results queued behind it", async () => {
    const { s, io, read } = fakeStore([{ id: "a", title: "old" }], 2);
    const bridge = createMeetingsBridge(io);
    const v = fakeView([], null);
    const token = bridge.attach(v.port); // still loading
    const pending = bridge.deliverPatch("a", { processed: "notes" });
    // The view goes away holding an unsaved rename.
    bridge.detach(token, JSON.stringify([{ id: "a", title: "renamed" }]));
    expect(await pending).toBe("disk");
    await bridge.idle();
    expect(read()).toEqual([{ id: "a", title: "renamed", processed: "notes" }]);
    expect(s.log).toEqual(["save", "load", "save"]);
  });

  test("a stale view's detach cannot unhook the view that replaced it", async () => {
    const { s, io } = fakeStore([{ id: "a" }]);
    const bridge = createMeetingsBridge(io);
    const old = bridge.attach(fakeView([]).port);
    const v2 = fakeView([{ id: "a" }]);
    bridge.markReady(bridge.attach(v2.port));
    bridge.detach(old); // e.g. React StrictMode, or an async cleanup running late
    expect(await bridge.deliverPatch("a", { x: 1 })).toBe("view");
    expect(s.log).toEqual([]);
  });

  test("a new meeting with no view is prepended once", async () => {
    const { io, read } = fakeStore([{ id: "old" }]);
    const bridge = createMeetingsBridge(io);
    expect(await bridge.deliverNew({ id: "new" }, true)).toBe("disk");
    expect(await bridge.deliverNew({ id: "new" }, true)).toBe("disk");
    expect(read().map((m) => m.id)).toEqual(["new", "old"]);
  });

  test("a new meeting reaches a ready view and is selected", async () => {
    const { s, io } = fakeStore([]);
    const bridge = createMeetingsBridge(io);
    const v = fakeView([]);
    bridge.markReady(bridge.attach(v.port));
    expect(await bridge.deliverNew({ id: "m1" }, true)).toBe("shown");
    expect(v.got).toEqual(["add:m1"]);
    expect(s.log).toEqual([]);
  });

  test("a first-run empty store accepts a new meeting", async () => {
    const { s, io, read } = fakeStore([]);
    s.doc = "";
    const bridge = createMeetingsBridge(io);
    expect(await bridge.deliverNew({ id: "first" }, true)).toBe("disk");
    expect(read()).toEqual([{ id: "first" }]);
  });

  test("a view that throws does not lose the result", async () => {
    const { io, read } = fakeStore([{ id: "a" }]);
    const bridge = createMeetingsBridge(io);
    bridge.markReady(
      bridge.attach({
        applyPatch: () => {
          throw new Error("boom");
        },
        addMeeting: () => {
          throw new Error("boom");
        },
      }),
    );
    expect(await bridge.deliverPatch("a", { processed: "kept" })).toBe("disk");
    expect(read()[0]).toEqual({ id: "a", processed: "kept" });
  });
});
