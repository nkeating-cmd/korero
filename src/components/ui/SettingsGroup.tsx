import React from "react";

interface SettingsGroupProps {
  title?: string;
  description?: string;
  children: React.ReactNode;
}

/**
 * Kōrero 1.42: settings groups sit on the same opaque card as everything
 * else (kx system). The overline title and hairline dividers match Today,
 * Meetings and the Activity panel, so every page reads as one product.
 */
export const SettingsGroup: React.FC<SettingsGroupProps> = ({
  title,
  description,
  children,
}) => {
  return (
    <section className="space-y-2">
      {title && (
        <div className="px-1">
          <h2 className="kx-overline">{title}</h2>
          {description && <p className="kx-meta mt-1">{description}</p>}
        </div>
      )}
      <div className="kx-card overflow-visible p-1">
        <div className="kx-divide">{children}</div>
      </div>
    </section>
  );
};
