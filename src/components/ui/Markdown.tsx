/* eslint-disable i18next/no-literal-string */
import React, { Suspense } from "react";

/**
 * Kōrero fork (v1.13.4): lazy-loaded markdown renderer. react-markdown +
 * remark-gfm only load when a rendered note is actually on screen; until
 * then they stay out of the main bundle. Style with the `.md-body` class on
 * a wrapping element (see App.css).
 */
const Inner = React.lazy(() => import("./MarkdownInner"));

export const Markdown: React.FC<{ children: string }> = ({ children }) => (
  <Suspense
    fallback={<div className="text-sm text-text-subtle">Rendering…</div>}
  >
    <Inner>{children}</Inner>
  </Suspense>
);
