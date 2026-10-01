import React from "react";
import { fold, foldWithMap } from "../../lib/search";

/**
 * Text with every occurrence of `query` marked, macron-insensitive
 * ("whanau" marks "whānau"). The original characters are kept.
 */
export const Highlighted: React.FC<{ text: string; query?: string }> = ({ text, query }) => {
  const q = fold(query ?? "").trim();
  if (!q) return <>{text}</>;
  const { folded, map } = foldWithMap(text);
  const parts: React.ReactNode[] = [];
  let cursor = 0;
  let at = folded.indexOf(q);
  let key = 0;
  while (at !== -1) {
    const start = map[at];
    const lastOrig = map[at + q.length - 1];
    const cp = text.codePointAt(lastOrig) ?? 0;
    const end = lastOrig + (cp > 0xffff ? 2 : 1);
    if (start > cursor) parts.push(text.slice(cursor, start));
    parts.push(
      <mark key={key++} className="kx-mark">
        {text.slice(start, end)}
      </mark>,
    );
    cursor = end;
    at = folded.indexOf(q, at + q.length);
  }
  if (cursor < text.length) parts.push(text.slice(cursor));
  return <>{parts}</>;
};

/** True when `text` contains `query`, macron-insensitive. */
export const containsFolded = (text: string, query: string): boolean => {
  const q = fold(query).trim();
  return !!q && fold(text).includes(q);
};
