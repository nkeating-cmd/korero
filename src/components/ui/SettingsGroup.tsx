import React from "react";

interface SettingsGroupProps {
  title?: string;
  description?: string;
  children: React.ReactNode;
}

/**
 * Kōrero fork: replaced upstream's solid-bg card with a frosted glass-card
 * surface matching the Daily Brief Dashboard aesthetic.
 * - .glass-card adds backdrop-filter blur + 1px white border at 10% opacity.
 * - Section title remains uppercase letter-spaced muted text.
 */
export const SettingsGroup: React.FC<SettingsGroupProps> = ({
  title,
  description,
  children,
}) => {
  return (
    <div className="space-y-2">
      {title && (
        <div className="px-4">
          <h2 className="text-xs font-semibold text-text-muted uppercase tracking-wider">
            {title}
          </h2>
          {description && (
            <p className="text-xs text-text-subtle mt-1">{description}</p>
          )}
        </div>
      )}
      {/* Kōrero fork: glass-card wrapper with no inner padding so child
          SettingContainer components retain their own px-4 p-2 layout.
          Forcing padding via [&>*] broke alignment for stacked-layout settings. */}
      <div className="glass-card overflow-visible p-1.5">
        <div className="divide-y divide-glass-border">{children}</div>
      </div>
    </div>
  );
};
