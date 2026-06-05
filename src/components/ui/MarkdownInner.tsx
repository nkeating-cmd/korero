import React from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * Kōrero fork (v1.13.4): the actual markdown renderer, isolated in its own
 * module so React.lazy can split react-markdown + remark-gfm (~180 KB) out of
 * the main bundle. Import via `ui/Markdown`, not directly.
 */
const MarkdownInner: React.FC<{ children: string }> = ({ children }) => (
  <ReactMarkdown remarkPlugins={[remarkGfm]}>{children}</ReactMarkdown>
);

export default MarkdownInner;
