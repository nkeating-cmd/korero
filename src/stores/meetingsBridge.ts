// Kōrero (meetings reliability, 2026-09-25): ONE route for meeting results
// that arrive after the work that produced them has outlived its view.
//
// WHY. Long meeting work (transcription, imports, notes) finishes minutes after
// it starts, and App.tsx unmounts the Meetings view whenever you change tab.
// v1.30.2 handled "the view is gone" by checking the STARTING view's own
// `mountedRef` and, if it was false, writing a patch straight to meetings.json.
// That check asked the wrong question. If you left and came BACK before the
// work finished, a NEW view was mounted — holding the list it had read from
// disk before the write — so the next autosave of that view wrote its stale
// copy over the patch. The transcript reverted, or an import vanished, with no
// error anywhere. The question that matters is "is ANY Meetings view able to
// take this right now?", and only something that outlives every view can
// answer it.
//
// THE RULES this module enforces:
//   1. A result goes to the mounted view if one is ready (its autosave then
//      persists it), to a view that is still LOADING once its load finishes,
//      and to disk only when no view exists.
//   2. Every disk read and write — the view's load, its autosaves, its final
//      save on unmount, and background patches — runs through one FIFO queue,
//      so a view never reads a file that a queued patch is about to change,
//      and a patch never lands under a save that was already on its way.
//   3. Only a store that was successfully READ is ever written back (the
//      v1.29.0 R-02 rule): a locked or unreadable meetings.json can never be
//      replaced by an empty or partial document.
//
// Deliberately free of Tauri imports: the IO is injected, so the ordering rules
// above are unit-tested (tests/unit/meetingsBridge.test.ts) without a webview.

export type MeetingPatch = Record<string, unknown>;
export type MeetingDoc = Record<string, unknown> & { id: string };

/** Where a delivered result ended up. */
export type DeliveryOutcome =
  | "shown" // applied to the mounted view, and it is the meeting on screen
  | "view" // applied to the mounted view (another meeting is on screen)
  | "disk" // no view — written to meetings.json
  | "missing" // the meeting no longer exists (deleted while the work ran)
  | "failed"; // no view, and the store could not be read or written

export interface MeetingsViewPort {
  /**
   * Apply a patch to the view's in-memory list. Returns "shown" when that
   * meeting is the one on screen, "view" when it is elsewhere in the list, and
   * "missing" when it is not in the list at all.
   */
  applyPatch(id: string, patch: MeetingPatch): "shown" | "view" | "missing";
  /** Put a brand-new meeting at the top of the list, optionally selecting it. */
  addMeeting(meeting: MeetingDoc, select: boolean): "shown" | "view";
}

export type LoadResult =
  | { ok: true; data: string }
  | { ok: false; error: string };
export type SaveResult = { ok: true } | { ok: false; error: string };

export interface StoreIO {
  load(): Promise<LoadResult>;
  save(json: string): Promise<SaveResult>;
}

export interface MeetingsBridge {
  /** A view mounted. Returns a token that identifies it in later calls. */
  attach(port: MeetingsViewPort): number;
  /** The view finished loading and has put the list into its state. */
  markReady(token: number): void;
  /**
   * The view is going away (or could not load). `finalSave`, if given, is its
   * last unsaved document; it is written BEFORE any queued background result.
   */
  detach(token: number, finalSave?: string): void;
  /** Read meetings.json, in queue order. */
  load(): Promise<LoadResult>;
  /** Write meetings.json, in queue order. */
  save(json: string): Promise<SaveResult>;
  /** Deliver a patch to an existing meeting (see the rules above). */
  deliverPatch(id: string, patch: MeetingPatch): Promise<DeliveryOutcome>;
  /** Deliver a brand-new meeting. `select` asks a mounted view to show it. */
  deliverNew(meeting: MeetingDoc, select: boolean): Promise<DeliveryOutcome>;
  /** Resolves once every queued disk operation so far has finished. */
  idle(): Promise<void>;
}

export function createMeetingsBridge(io: StoreIO): MeetingsBridge {
  let port: MeetingsViewPort | null = null;
  let token = 0;
  let ready = false;
  // Deliveries that arrived while the view was still loading. Each entry
  // re-decides its route when run, so the same closure works whether the view
  // becomes ready or goes away first.
  let waiting: Array<() => void> = [];
  let chain: Promise<unknown> = Promise.resolve();

  const serial = <T>(fn: () => Promise<T>): Promise<T> => {
    const run = chain.then(fn, fn);
    chain = run.catch(() => undefined);
    return run;
  };

  const readList = async (): Promise<
    { ok: true; list: MeetingDoc[] } | { ok: false }
  > => {
    const res = await io.load();
    if (!res.ok) return { ok: false };
    if (!res.data.trim()) return { ok: true, list: [] };
    try {
      const parsed = JSON.parse(res.data) as unknown;
      return Array.isArray(parsed)
        ? { ok: true, list: parsed as MeetingDoc[] }
        : { ok: false };
    } catch {
      return { ok: false };
    }
  };

  const patchOnDisk = async (
    id: string,
    patch: MeetingPatch,
  ): Promise<DeliveryOutcome> => {
    const read = await readList();
    if (!read.ok) return "failed";
    let found = false;
    const next = read.list.map((m) => {
      if (m && m.id === id) {
        found = true;
        return { ...m, ...patch };
      }
      return m;
    });
    if (!found) return "missing";
    const saved = await io.save(JSON.stringify(next));
    return saved.ok ? "disk" : "failed";
  };

  const addOnDisk = async (meeting: MeetingDoc): Promise<DeliveryOutcome> => {
    const read = await readList();
    if (!read.ok) return "failed";
    if (read.list.some((m) => m && m.id === meeting.id)) return "disk";
    const saved = await io.save(JSON.stringify([meeting, ...read.list]));
    return saved.ok ? "disk" : "failed";
  };

  const runWaiting = () => {
    const q = waiting;
    waiting = [];
    q.forEach((f) => f());
  };

  const route = (
    toView: (p: MeetingsViewPort) => DeliveryOutcome,
    toDisk: () => Promise<DeliveryOutcome>,
  ): Promise<DeliveryOutcome> =>
    new Promise((resolve) => {
      const attempt = () => {
        if (port && ready) {
          try {
            resolve(toView(port));
          } catch {
            // A throwing view must not lose the result: fall back to disk.
            serial(toDisk).then(resolve, () => resolve("failed"));
          }
          return;
        }
        if (port && !ready) {
          waiting.push(attempt);
          return;
        }
        serial(toDisk).then(resolve, () => resolve("failed"));
      };
      attempt();
    });

  return {
    attach(p) {
      token += 1;
      port = p;
      ready = false;
      return token;
    },
    markReady(t) {
      if (t !== token || !port) return;
      ready = true;
      runWaiting();
    },
    detach(t, finalSave) {
      if (t !== token) return; // a newer view has already taken over
      port = null;
      ready = false;
      if (finalSave !== undefined) void serial(() => io.save(finalSave));
      runWaiting(); // no view now: each waiting delivery routes to disk
    },
    load: () => serial(() => io.load()),
    save: (json) => serial(() => io.save(json)),
    deliverPatch: (id, patch) =>
      route(
        (p) => p.applyPatch(id, patch),
        () => patchOnDisk(id, patch),
      ),
    deliverNew: (meeting, select) =>
      route(
        (p) => p.addMeeting(meeting, select),
        () => addOnDisk(meeting),
      ),
    idle: () => serial(async () => undefined),
  };
}
