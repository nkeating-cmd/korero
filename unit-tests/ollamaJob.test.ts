// Kōrero 1.43: the "Download and use" job. Run with `bun test unit-tests`.

import { describe, expect, mock, test } from "bun:test";

// The store module imports Tauri and the settings store; the pure parts don't
// need them, so stub those modules before importing.
mock.module("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
mock.module("@/bindings", () => ({ commands: {} }));
mock.module("../src/stores/settingsStore", () => ({ useSettingsStore: { getState: () => ({ settings: null }) } }));

const { CANCELLED, emptyProgress, etaSeconds, ok, reduceProgress, runChain } = await import("../src/stores/ollamaJobStore");
type Deps = Parameters<typeof runChain>[1];

describe("reduceProgress (RT-F-10)", () => {
  test("sums layers and never goes backwards", () => {
    let p = emptyProgress(0);
    p = reduceProgress(p, { status: "pulling", digest: "a", total: 100, completed: 100 }, 100);
    p = reduceProgress(p, { status: "pulling", digest: "b", total: 900, completed: 0 }, 200);
    expect(p.totalBytes).toBe(1000);
    expect(p.doneBytes).toBe(100);
    p = reduceProgress(p, { status: "pulling", digest: "b", total: 900, completed: 450 }, 300);
    expect(p.doneBytes).toBe(550);
    // A stale lower figure for a layer never moves the bar back.
    p = reduceProgress(p, { status: "pulling", digest: "b", total: 900, completed: 10 }, 400);
    expect(p.doneBytes).toBe(550);
  });

  test("a resumed layer doesn't spike the rate", () => {
    let p = emptyProgress(0);
    // First sight of a layer already 4 GB in: resumed, not downloaded now.
    p = reduceProgress(p, { status: "pulling", digest: "a", total: 5e9, completed: 4e9 }, 1);
    p = reduceProgress(p, { status: "pulling", digest: "a", total: 5e9, completed: 4e9 + 10e6 }, 1001);
    expect(p.rateBps).not.toBeNull();
    expect(p.rateBps!).toBeLessThan(20e6);
    expect(etaSeconds(p)).toBeGreaterThan(50);
  });

  test("status-only lines keep the bytes", () => {
    let p = reduceProgress(emptyProgress(0), { status: "pulling", digest: "a", total: 10, completed: 5 }, 1);
    p = reduceProgress(p, { status: "verifying sha256 digest" }, 2);
    expect(p.doneBytes).toBe(5);
    expect(p.status).toBe("verifying sha256 digest");
  });
});

function deps(over: Partial<Deps> = {}): { d: Deps; steps: string[]; used: string[] } {
  const steps: string[] = [];
  const used: string[] = [];
  const d: Deps = {
    status: async () => ({ installed: true, running: true }),
    start: async () => true,
    pull: async () => {},
    test: async () => ({ seconds: 1.2, sample: "Kia ora.", gpu_share: 1 }),
    use: async (m) => {
      used.push(m);
    },
    snapshot: () => "same",
    onStep: (s) => steps.push(s),
    ...over,
  };
  return { d, steps, used };
}

describe("runChain", () => {
  test("happy path: pull, test, switch", async () => {
    const { d, steps, used } = deps();
    await runChain("qwen3.5:4b", d);
    expect(steps).toEqual(["pulling", "testing", "switching", "done"]);
    expect(used).toEqual(["qwen3.5:4b"]);
  });

  test("starts Ollama when it isn't running", async () => {
    const { d, steps } = deps({ status: async () => ({ installed: true, running: false }) });
    await runChain("m", d);
    expect(steps[0]).toBe("starting");
  });

  test("a failed pull never switches (RT-F-01)", async () => {
    const { d, steps, used } = deps({
      pull: async () => {
        throw new Error("Ollama couldn't download it: file does not exist");
      },
    });
    await runChain("m", d);
    expect(steps.at(-1)).toBe("error");
    expect(used).toEqual([]);
  });

  test("a failed test run never switches", async () => {
    const { d, steps, used } = deps({
      test: async () => {
        throw new Error("Ollama can't use m (HTTP 500): unsupported tensor");
      },
    });
    await runChain("m", d);
    expect(steps.at(-1)).toBe("error");
    expect(used).toEqual([]);
  });

  test("cancel ends as cancelled, not as an error", async () => {
    const { d, steps, used } = deps({
      pull: async () => {
        throw new Error(CANCELLED);
      },
    });
    await runChain("m", d);
    expect(steps.at(-1)).toBe("cancelled");
    expect(used).toEqual([]);
  });

  test("settings changed during the download: no switch (RT-F-04)", async () => {
    let n = 0;
    const { d, steps, used } = deps({ snapshot: () => (n++ === 0 ? "before" : "after") });
    await runChain("m", d);
    expect(steps.at(-1)).toBe("done");
    expect(used).toEqual([]);
  });

  test("not installed stops before anything else", async () => {
    const { d, steps } = deps({ status: async () => ({ installed: false, running: false }) });
    await runChain("m", d);
    expect(steps).toEqual(["error"]);
  });

  test("ok() turns an error value into a throw", () => {
    expect(ok({ status: "ok", data: 3 })).toBe(3);
    expect(() => ok({ status: "error", error: "nope" })).toThrow("nope");
  });
});
