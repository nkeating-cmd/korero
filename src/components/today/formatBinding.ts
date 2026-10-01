/** "ctrl+shift+space" → "Ctrl Shift Space". Empty for no binding. */
export const formatBinding = (binding: string | null | undefined): string => {
  if (!binding) return "";
  return binding
    .split("+")
    .map((p) => p.trim())
    .filter(Boolean)
    .map((p) => {
      const k = p.toLowerCase();
      if (k === "ctrl" || k === "control") return "Ctrl";
      if (k === "cmd" || k === "command" || k === "super" || k === "meta") return "Win";
      if (k === "alt" || k === "option") return "Alt";
      if (k === "shift") return "Shift";
      if (k === "space") return "Space";
      if (k === "enter" || k === "return") return "Enter";
      if (k === "escape" || k === "esc") return "Esc";
      return p.length === 1 ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1);
    })
    .join(" ");
};
