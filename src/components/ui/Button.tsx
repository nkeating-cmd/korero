import React from "react";

/**
 * Kōrero 1.42: the one button, drawn by the `kx-btn` system (src/styles/kx.css).
 *
 * The variant names are kept so the ~120 existing call sites change look
 * without being touched. `primary` now fills with #0b63ce, which passes AA
 * under white text (5.7:1); the old fill measured 3.65:1.
 */
interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?:
    | "primary"
    | "primary-soft"
    | "secondary"
    | "danger"
    | "danger-ghost"
    | "ghost"
    | "quiet";
  size?: "sm" | "md" | "lg";
}

const VARIANT: Record<NonNullable<ButtonProps["variant"]>, string> = {
  primary: "kx-btn-primary",
  "primary-soft": "kx-btn-quiet",
  quiet: "kx-btn-quiet",
  secondary: "kx-btn-secondary",
  danger: "kx-btn-danger",
  "danger-ghost": "kx-btn-ghost kx-danger-ghost",
  ghost: "kx-btn-ghost",
};

const SIZE: Record<NonNullable<ButtonProps["size"]>, string> = {
  sm: "kx-btn-sm",
  md: "",
  lg: "",
};

export const Button: React.FC<ButtonProps> = ({
  children,
  className = "",
  variant = "primary",
  size = "md",
  type = "button",
  ...props
}) => (
  <button
    type={type}
    className={`kx-btn ${VARIANT[variant]} ${SIZE[size]} ${className}`}
    {...props}
  >
    {children}
  </button>
);
