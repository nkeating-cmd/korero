import React from "react";

interface BadgeProps {
  children: React.ReactNode;
  variant?: "primary" | "success" | "secondary";
  className?: string;
}

/**
 * Korero fork: badges become glass pills with aurora-cyan accent for primary.
 */
const Badge: React.FC<BadgeProps> = ({ children, variant = "primary", className = "" }) => {
  const variantClasses = {
    primary: "glass-pill glass-pill-accent",
    success: "glass-pill pill-positive",
    secondary: "glass-pill text-text-muted",
  };
  return <span className={`${variantClasses[variant]} ${className}`}>{children}</span>;
};

export default Badge;
