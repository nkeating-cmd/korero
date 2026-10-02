// Kōrero 1.43: which local model fits which computer. Run with `bun test unit-tests`.

import { describe, expect, test } from "bun:test";
import { CATALOGUE } from "../src/lib/modelCatalogue";
import { fitFor, gpuSummary, recommend, sortByFit, type ProfileLike } from "../src/lib/modelFit";

const GB = 1024;
const gpu = (vramGb: number, ramGb: number, diskGb: number | null = 400): ProfileLike => ({
  gpus: [
    { name: "Discrete GPU", vram_mb: vramGb * GB, kind: "discrete" },
    { name: "Intel(R) UHD Graphics", vram_mb: 128, kind: "integrated" },
  ],
  ram_mb: ramGb * GB,
  free_disk_mb: diskGb === null ? null : diskGb * GB,
});
const cpuOnly = (ramGb: number): ProfileLike => ({
  gpus: [{ name: "Intel(R) UHD Graphics", vram_mb: 128, kind: "integrated" }],
  ram_mb: ramGb * GB,
  free_disk_mb: 200 * GB,
});
const mac = (ramGb: number): ProfileLike => ({
  gpus: [{ name: "Apple Silicon (unified memory)", vram_mb: ramGb * GB, kind: "apple" }],
  ram_mb: ramGb * GB,
  free_disk_mb: 300 * GB,
});
const entry = (tag: string) => CATALOGUE.find((e) => e.tag === tag)!;

describe("recommend", () => {
  test("8 GB graphics card: the largest model that runs fully on it", () =>
    expect(recommend(CATALOGUE, gpu(8, 16))?.tag).toBe("qwen3.5:4b"));
  test("12 GB graphics card (the test laptop): gemma4:12b", () => expect(recommend(CATALOGUE, gpu(12, 32))?.tag).toBe("gemma4:12b"));
  test("24 GB graphics card: 26b only just fits, so the 12b is suggested and the 26b is offered", () => {
    expect(recommend(CATALOGUE, gpu(24, 64))?.tag).toBe("gemma4:12b");
    expect(fitFor(entry("gemma4:26b"), gpu(24, 64)).fit).toBe("tight");
  });
  test("32 GB graphics card: gemma4:26b", () => expect(recommend(CATALOGUE, gpu(32, 64))?.tag).toBe("gemma4:26b"));
  test("no graphics card: the smallest model, on the processor", () =>
    expect(recommend(CATALOGUE, cpuOnly(16))?.tag).toBe("qwen3.5:2b"));
  test("nothing fits: no recommendation", () => expect(recommend(CATALOGUE, cpuOnly(4))).toBeNull());
  test("Apple Silicon 24 GB", () => expect(recommend(CATALOGUE, mac(24))?.tag).toBe("gemma4:12b"));
});

describe("fitFor", () => {
  test("an integrated GPU never counts as graphics memory", () =>
    expect(fitFor(entry("qwen3.5:2b"), cpuOnly(16)).fit).toBe("cpu"));
  test("too big for GPU plus memory is refused", () => expect(fitFor(entry("gemma4:26b"), gpu(8, 16)).fit).toBe("no"));
  test("not enough disk is refused, whatever the GPU", () => {
    const f = fitFor(entry("qwen3.5:4b"), gpu(24, 64, 4));
    expect(f.fit).toBe("no");
    expect(f.reason).toContain("disk");
  });
  test("disk unknown skips the disk rule (RT-F-11)", () =>
    expect(fitFor(entry("qwen3.5:4b"), gpu(8, 16, null)).fit).toBe("great"));
  test("spills onto the processor when the GPU is too small", () =>
    expect(fitFor(entry("gemma4:12b"), gpu(8, 16)).fit).toBe("cpu"));
  test("an 8 GB Mac is not offered a 9 GB model (RT-A-08)", () => {
    expect(fitFor(entry("qwen3.5:9b"), mac(8)).fit).toBe("no");
    expect(fitFor(entry("qwen3.5:2b"), mac(8)).fit).toBe("great");
  });
  test("an AMD APU classed integrated by Rust runs on the processor", () => {
    const apu: ProfileLike = {
      gpus: [{ name: "AMD Radeon(TM) Graphics", vram_mb: 4 * GB, kind: "integrated" }],
      ram_mb: 32 * GB,
      free_disk_mb: 100 * GB,
    };
    expect(fitFor(entry("qwen3.5:4b"), apu).fit).toBe("cpu");
  });
  test("sized for a 32k context (RT-A-07): the allowance grows with the model", () => {
    expect(fitFor(entry("gemma4:12b"), gpu(12, 32)).needMb).toBeGreaterThan(entry("gemma4:12b").downloadMb + 2000);
  });
});

describe("ordering and summary", () => {
  test("best fit first, then smallest", () => {
    const order = sortByFit(CATALOGUE, gpu(8, 16)).map((e) => e.tag);
    expect(order[0]).toBe("qwen3.5:2b");
    expect(order[order.length - 1]).toBe("gemma4:26b");
  });
  test("summary names the best discrete card", () => {
    expect(gpuSummary(gpu(12, 32))).toBe("Discrete GPU · 12 GB");
    expect(gpuSummary(cpuOnly(16))).toContain("processor");
  });
  test("every entry is marked provisional until the bake-off", () =>
    expect(CATALOGUE.every((e) => e.provisional)).toBe(true));
});
