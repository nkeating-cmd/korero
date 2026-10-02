/**
 * Kōrero 1.43: which local AI model fits this computer.
 *
 * Pure functions over the Rust `get_machine_profile` result (sizes in MB), so
 * the rules are unit-tested (unit-tests/modelFit.test.ts) and the UI only
 * renders.
 *
 * Memory needed = the download (≈ the weights) + an allowance for the context
 * cache and runtime, sized for the largest context the app asks for (32k
 * tokens, ollama_chat::MAX_CTX). Calibrated on the test laptop: gemma4:12b stays at
 * 7.54–7.81 GiB resident from 8k to 32k context, so these allowances are
 * conservative for it. Other families are not measured; the test run reports
 * the real GPU share after download.
 */

import type { CatalogueEntry } from "./modelCatalogue";

export interface GpuLike {
  name: string;
  vram_mb: number;
  kind: "discrete" | "integrated" | "apple";
}

export interface ProfileLike {
  gpus: GpuLike[];
  ram_mb: number;
  free_disk_mb: number | null;
}

export type Fit = "great" | "tight" | "cpu" | "no";

export interface FitResult {
  fit: Fit;
  needMb: number;
  reason: string;
}

/** Context cache and runtime on top of the weights, at a 32k context. */
export const overheadMb = (downloadMb: number): number =>
  downloadMb <= 5 * 1024 ? 1536 : downloadMb <= 10 * 1024 ? 2048 : 3072;

const isApple = (p: ProfileLike) => p.gpus.some((g) => g.kind === "apple");

/** Memory a model can run in at full speed. */
export function fastMemoryMb(p: ProfileLike): number {
  // Apple Silicon: one pool shared with macOS and every app; cap the model at
  // three-quarters of it, and never count it twice.
  if (isApple(p)) return Math.floor(p.ram_mb * 0.75);
  return Math.max(0, ...p.gpus.filter((g) => g.kind === "discrete").map((g) => g.vram_mb));
}

const gbText = (mb: number) => `${Math.ceil(mb / 1024)} GB`;

export function fitFor(e: CatalogueEntry, p: ProfileLike): FitResult {
  const needMb = e.downloadMb + overheadMb(e.downloadMb);
  if (p.free_disk_mb !== null && p.free_disk_mb < e.downloadMb + 2048) {
    return { fit: "no", needMb, reason: `Needs ${gbText(e.downloadMb + 2048)} of free disk space` };
  }
  const fast = fastMemoryMb(p);
  if (fast > 0 && needMb <= fast * 0.9) return { fit: "great", needMb, reason: "Runs fully on your graphics card" };
  if (fast > 0 && needMb <= fast) return { fit: "tight", needMb, reason: "Fits, with little room to spare" };
  if (!isApple(p) && needMb <= fast + p.ram_mb * 0.6) {
    return {
      fit: "cpu",
      needMb,
      reason:
        fast > 0
          ? "Part runs on the processor, so meeting notes take longer"
          : "Runs on the processor: fine for short dictation, slow for meeting notes",
    };
  }
  return { fit: "no", needMb, reason: `Won't fit: needs about ${gbText(needMb)} of memory` };
}

const rank: Record<Fit, number> = { great: 3, tight: 2, cpu: 1, no: 0 };

/**
 * The one model to suggest: the largest that runs fully on the graphics card;
 * else the largest that fits at all; else the smallest that runs on the
 * processor; else nothing.
 */
export function recommend(catalogue: CatalogueEntry[], p: ProfileLike): CatalogueEntry | null {
  const scored = catalogue.map((e) => ({ e, f: fitFor(e, p).fit }));
  const largest = (fit: Fit) =>
    scored.filter((s) => s.f === fit).sort((a, b) => b.e.downloadMb - a.e.downloadMb)[0]?.e;
  const smallestCpu = scored.filter((s) => s.f === "cpu").sort((a, b) => a.e.downloadMb - b.e.downloadMb)[0]?.e;
  return largest("great") ?? largest("tight") ?? smallestCpu ?? null;
}

export const sortByFit = (catalogue: CatalogueEntry[], p: ProfileLike): CatalogueEntry[] =>
  [...catalogue].sort((a, b) => rank[fitFor(b, p).fit] - rank[fitFor(a, p).fit] || a.downloadMb - b.downloadMb);

/** "NVIDIA GeForce RTX 4060 Laptop GPU · 8 GB", or why there isn't one. */
export function gpuSummary(p: ProfileLike): string {
  const apple = p.gpus.find((g) => g.kind === "apple");
  if (apple) return `${apple.name} · ${gbText(p.ram_mb)}`;
  const best = [...p.gpus].filter((g) => g.kind === "discrete").sort((a, b) => b.vram_mb - a.vram_mb)[0];
  if (best) return `${best.name} · ${gbText(best.vram_mb)}`;
  return "No graphics card for AI: models run on the processor";
}
