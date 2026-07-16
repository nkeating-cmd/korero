// Korero fork (v1.2.0): portal-based dropdown list.
//
// Problem: the settings page is a scrollable container. Position-absolute
// dropdowns get clipped by the nearest ancestor with overflow != visible
// (the settings scroll pane). With 11 providers the list was cut off.
//
// Fix: render the dropdown list via ReactDOM.createPortal at document.body,
// positioned with `position: fixed` and coordinates from getBoundingClientRect().
// This bypasses ALL ancestor overflow constraints. On scroll/resize the position
// is recalculated and the dropdown closes if the trigger scrolls off-screen.

import React, { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";

export interface DropdownOption {
  value: string;
  label: string;
  disabled?: boolean;
}

interface DropdownProps {
  options: DropdownOption[];
  className?: string;
  selectedValue: string | null;
  onSelect: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  onRefresh?: () => void;
}

interface DropdownPos {
  top: number;
  left: number;
  width: number;
}

export const Dropdown: React.FC<DropdownProps> = ({
  options,
  selectedValue,
  onSelect,
  className = "",
  placeholder = "Select an option...",
  disabled = false,
  onRefresh,
}) => {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const [dropdownPos, setDropdownPos] = useState<DropdownPos | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // Close on outside click
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      const target = event.target as Node;
      if (triggerRef.current?.contains(target)) return;
      if (listRef.current?.contains(target)) return;
      setIsOpen(false);
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  // Calculate / update portal position while open
  useEffect(() => {
    if (!isOpen) return;

    const reposition = () => {
      if (!triggerRef.current) return;
      const rect = triggerRef.current.getBoundingClientRect();
      // If the trigger has scrolled fully out of viewport, close the dropdown
      if (rect.bottom < 0 || rect.top > window.innerHeight) {
        setIsOpen(false);
        return;
      }
      setDropdownPos({ top: rect.bottom + 4, left: rect.left, width: rect.width });
    };

    reposition(); // initial position

    // Capture-phase scroll catches nested scroll containers (the settings pane)
    window.addEventListener("scroll", reposition, true);
    window.addEventListener("resize", reposition);
    return () => {
      window.removeEventListener("scroll", reposition, true);
      window.removeEventListener("resize", reposition);
    };
  }, [isOpen]);

  const selectedOption = options.find((o) => o.value === selectedValue);

  const handleSelect = (value: string) => {
    onSelect(value);
    setIsOpen(false);
    // v1.25.0 (a11y): hand focus back to the trigger so keyboard users
    // aren't dropped at document body after choosing.
    requestAnimationFrame(() => triggerRef.current?.focus());
  };

  const handleToggle = () => {
    if (disabled) return;
    if (!isOpen && onRefresh) onRefresh();
    setIsOpen(!isOpen);
  };

  // v1.25.0 (a11y): keyboard navigation. The options are real <button>s, so
  // Enter/Space already select — this adds Arrow/Home/End roving focus inside
  // the portal list, Escape-to-close, and open-from-keyboard on the trigger.
  const focusOption = (dir: "first" | "last" | "next" | "prev") => {
    const list = listRef.current;
    if (!list) return;
    const items = Array.from(
      list.querySelectorAll<HTMLButtonElement>("button:not([disabled])"),
    );
    if (items.length === 0) return;
    const current = items.indexOf(document.activeElement as HTMLButtonElement);
    let target = 0;
    if (dir === "first") target = 0;
    else if (dir === "last") target = items.length - 1;
    else if (dir === "next") target = current < items.length - 1 ? current + 1 : 0;
    else target = current > 0 ? current - 1 : items.length - 1;
    items[target]?.focus();
  };

  // Review fix (v1.25.0 #1): on the FIRST keyboard-open the portal hasn't
  // mounted yet (dropdownPos is set by a passive effect), so an immediate
  // focus call hits a null listRef. Park the intent and consume it from an
  // effect that fires once the portal is actually in the DOM.
  const pendingFocusRef = useRef<"first" | "last" | null>(null);
  useEffect(() => {
    if (isOpen && dropdownPos && pendingFocusRef.current) {
      const dir = pendingFocusRef.current;
      pendingFocusRef.current = null;
      requestAnimationFrame(() => focusOption(dir));
    }
  }, [isOpen, dropdownPos]);

  const handleTriggerKeyDown = (e: React.KeyboardEvent) => {
    if (disabled) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      pendingFocusRef.current = e.key === "ArrowUp" ? "last" : "first";
      if (!isOpen) {
        if (onRefresh) onRefresh();
        setIsOpen(true);
      } else {
        // Already open: the effect won't re-run, focus directly.
        pendingFocusRef.current = null;
        focusOption(e.key === "ArrowUp" ? "last" : "first");
      }
    } else if (e.key === "Escape" && isOpen) {
      setIsOpen(false);
    }
  };

  const handleListKeyDown = (e: React.KeyboardEvent) => {
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        focusOption("next");
        break;
      case "ArrowUp":
        e.preventDefault();
        focusOption("prev");
        break;
      case "Home":
        e.preventDefault();
        focusOption("first");
        break;
      case "End":
        e.preventDefault();
        focusOption("last");
        break;
      case "Escape":
        e.preventDefault();
        setIsOpen(false);
        triggerRef.current?.focus();
        break;
      case "Tab":
        // Tabbing out of a portal list is disorienting — close instead.
        setIsOpen(false);
        break;
      default:
        break;
    }
  };

  return (
    <div className={`relative ${className}`}>
      <button
        ref={triggerRef}
        type="button"
        className={`px-2.5 py-1.5 text-sm font-medium bg-glass-surface-thin border border-glass-border rounded-lg min-w-[200px] text-start flex items-center justify-between transition-all duration-150 ${
          disabled
            ? "opacity-50 cursor-not-allowed"
            : "hover:bg-glass-surface-hover hover:border-glass-border-strong cursor-pointer"
        }`}
        onClick={handleToggle}
        onKeyDown={handleTriggerKeyDown}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={isOpen}
      >
        <span className="truncate">{selectedOption?.label || placeholder}</span>
        <svg
          className={`w-4 h-4 ms-2 transition-transform duration-200 ${isOpen ? "transform rotate-180" : ""}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M19 9l-7 7-7-7"
          />
        </svg>
      </button>

      {isOpen && !disabled && dropdownPos &&
        createPortal(
          <div
            ref={listRef}
            role="listbox"
            onKeyDown={handleListKeyDown}
            style={{
              position: "fixed",
              top: dropdownPos.top,
              left: dropdownPos.left,
              width: dropdownPos.width,
              zIndex: 9999,
            }}
            className="bg-background border border-glass-border-strong rounded-lg shadow-xl max-h-60 overflow-y-auto"
          >
            {options.length === 0 ? (
              <div className="px-2.5 py-1.5 text-sm text-text-subtle">
                {t("common.noOptionsFound")}
              </div>
            ) : (
              options.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  role="option"
                  aria-selected={selectedValue === option.value}
                  className={`w-full px-2.5 py-1.5 text-sm text-start hover:bg-glass-surface-hover focus-visible:bg-glass-surface-hover transition-colors duration-150 ${
                    selectedValue === option.value
                      ? "bg-aurora-cyan/15 text-aurora-cyan font-medium"
                      : "text-text"
                  } ${option.disabled ? "opacity-50 cursor-not-allowed" : ""}`}
                  onClick={() => handleSelect(option.value)}
                  disabled={option.disabled}
                >
                  <span className="truncate">{option.label}</span>
                </button>
              ))
            )}
          </div>,
          document.body,
        )}
    </div>
  );
};
