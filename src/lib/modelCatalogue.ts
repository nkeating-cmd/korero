/**
 * Kōrero 1.43: local AI models the "Run AI clean-up on this computer" card
 * can offer, smallest first.
 *
 * PROVISIONAL. These are candidates, not tested picks: the clean-up bake-off
 * (NZ spelling, te reo kept with macrons, nothing invented, no preamble) has
 * not run on them yet. gemma4:12b is the one in daily use (the test laptop, thinking
 * off: fixed the "lime/line prices" error, 1 Oct 2026).
 *
 * Sizes are Ollama's default tags as listed on ollama.com on 2 Oct 2026
 * (gemma4:12b 7.7–8.0 GB, gemma4:26b 16–19 GB: the larger figure is used).
 * Review at each release: tags get re-pointed.
 */

export interface CatalogueEntry {
  tag: string;
  /** Download size in MB. */
  downloadMb: number;
  blurb: string;
  provisional: boolean;
}

export const CATALOGUE_VERIFIED = "2026-10-02";

const gb = (n: number) => Math.round(n * 1024);

export const CATALOGUE: CatalogueEntry[] = [
  { tag: "qwen3.5:2b", downloadMb: gb(2.7), blurb: "Smallest. For older or low-memory computers.", provisional: true },
  { tag: "qwen3.5:4b", downloadMb: gb(3.4), blurb: "Fast. Good for dictation clean-up and short notes.", provisional: true },
  { tag: "qwen3.5:9b", downloadMb: gb(6.6), blurb: "Balanced quality and speed.", provisional: true },
  { tag: "gemma4:12b", downloadMb: gb(8.0), blurb: "Strong meeting notes on a 12 GB graphics card.", provisional: true },
  { tag: "gemma4:26b", downloadMb: gb(19), blurb: "Highest quality. Needs a 24 GB graphics card.", provisional: true },
];
