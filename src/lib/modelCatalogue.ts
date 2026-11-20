/**
 * Kōrero 1.43: local AI models the "Run AI clean-up on this computer" card
 * can offer, smallest first.
 *
 * Kōrero 1.49: chosen from the clean-up bake-off of 20 Nov 2026
 * (docs/bakeoff/2026-11-20-catalogue.md: 16 fixtures x 3 runs on Kōrero's own
 * prompt). gemma4:12b and gemma4:26b were safe (40-42 of 48). The qwen3.5
 * models change words or turn sentences into lists, so they carry a caution.
 * qwen3.5:2b was dropped: it pasted Kōrero's own instructions back as the
 * answer, and it was the model recommended to every computer without a
 * graphics card (yoga-26, 20 Nov 2026).
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
  /** What the bake-off saw it get wrong, shown beside the blurb. */
  caution?: string;
}

/** The day the bake-off behind these picks ran. */
export const CATALOGUE_VERIFIED = "2026-11-20";

const gb = (n: number) => Math.round(n * 1024);

export const CATALOGUE: CatalogueEntry[] = [
  {
    tag: "qwen3.5:4b",
    downloadMb: gb(3.4),
    blurb: "Smallest. Fast enough for dictation on the processor.",
    caution: "In testing it sometimes changed a word or made a sentence into a list, so check what it pastes.",
  },
  {
    tag: "qwen3.5:9b",
    downloadMb: gb(6.6),
    blurb: "Balanced quality and speed.",
    caution: "In testing it sometimes made a plain sentence into a list.",
  },
  { tag: "gemma4:12b", downloadMb: gb(8.0), blurb: "The most careful in testing. Best on a 12 GB graphics card." },
  { tag: "gemma4:26b", downloadMb: gb(19), blurb: "As careful as the 12b, for meeting notes. Needs a 24 GB graphics card." },
];
