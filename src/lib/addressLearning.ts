/**
 * Kōrero 1.51: learning email addresses from the user's own edits.
 *
 * When the user corrects a dictation in History and replaces one email
 * address with another ("nick@kite.co.nz" → "nic@kyt.nz"), Kōrero offers to
 * remember it: a whole-address correction (so that exact mishearing is fixed
 * every time) plus the right address in "Your email addresses" (so near
 * misses that sound like it are fixed too, by `known_addresses.rs`).
 *
 * Only whole addresses are learnt. Word-level suggestions from edits that
 * rewrite sentences were too noisy ("costs → cost", "their → there"): the
 * 26 Nov 2026 red team, M5. Unit-tested in unit-tests/addressLearning.test.ts.
 */

const EMAIL = /[A-Za-z0-9][A-Za-z0-9._+-]*@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+/g;

/** Lower case, trimmed, and a well-formed address; null otherwise. */
export function normaliseAddress(s: string): string | null {
  const t = s.trim().replace(/^<|>$/g, "").toLowerCase();
  const m = t.match(/^[a-z0-9][a-z0-9._+-]*@[a-z0-9-]+(?:\.[a-z0-9-]+)+$/);
  return m ? t : null;
}

/** The email addresses in a text, lower case, in order (a trailing full stop is not part). */
export function findAddresses(text: string): string[] {
  return (text.match(EMAIL) ?? [])
    .map((a) => a.replace(/[.-]+$/, "").toLowerCase())
    .filter((a) => normaliseAddress(a) !== null);
}

export interface AddressChange {
  /** The address as Kōrero wrote it. */
  heard: string;
  /** The address the user put in its place. */
  means: string;
}

/**
 * Address replacements between the text before and after an edit. An address
 * that disappeared is paired with one that appeared, in order. Pairs where the
 * user already taught that exact correction are left out, and so are pairs
 * whose "heard" address is one of the user's own: correcting "nic@kyt.nz" to
 * a colleague's "nick@kyt.nz" must never teach Kōrero to rewrite the user's
 * own address (independent review, 1.51).
 */
export function addressChanges(
  before: string,
  after: string,
  taught: { wrong: string; right: string }[] = [],
  known: string[] = [],
): AddressChange[] {
  const was = findAddresses(before);
  const now = findAddresses(after);
  const gone = was.filter((a) => !now.includes(a));
  const added = now.filter((a) => !was.includes(a));
  const out: AddressChange[] = [];
  for (let i = 0; i < Math.min(gone.length, added.length); i++) {
    const heard = gone[i];
    const means = added[i];
    if (heard === means) continue;
    if (taught.some((c) => c.wrong.trim().toLowerCase() === heard)) continue;
    if (known.some((k) => k.trim().toLowerCase() === heard)) continue;
    out.push({ heard, means });
  }
  return out;
}
