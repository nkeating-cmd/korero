import i18next from "eslint-plugin-i18next";
import jsxA11y from "eslint-plugin-jsx-a11y";
import reactHooks from "eslint-plugin-react-hooks";
import tsParser from "@typescript-eslint/parser";

// =============================================================================
// Korero v1.31.0 -- accessibility linting, added as a RATCHET.
//
// Until now this config registered exactly ONE plugin. Hooks and accessibility
// had never been linted here, which is why the tree accumulated mouse-only
// rows, unlabelled form controls, and a reduced-motion rule that froze every
// progress indicator: nothing could ever have caught them.
//
// A one-shot "turn everything on" would have added 69 findings across 28 files
// and produced a permanently-red gate that everyone learns to ignore -- which
// is worse than no gate, because it looks like coverage. So the rules below sit
// in two bands:
//
//   ERROR -- cleared. These can never regress.
//   OFF   -- not yet cleared, each with its measured count and the fix.
//            Promote to "error" in the commit that clears it. Never promote
//            early, and never add a rule here without a count.
//
// Measured 2026-08-18 against jsx-a11y `recommended` + react-hooks.
// =============================================================================

export default [
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        ecmaFeatures: {
          jsx: true,
        },
      },
    },
    plugins: {
      i18next,
      "jsx-a11y": jsxA11y,
      "react-hooks": reactHooks,
    },
    rules: {
      // ---- ENFORCED from v1.31.0 ------------------------------------------
      // Hooks called conditionally are a correctness bug, not a style issue.
      "react-hooks/rules-of-hooks": "error",
      // Cleared: zero offenders today. These lock the door behind us.
      "jsx-a11y/alt-text": "error",
      "jsx-a11y/anchor-has-content": "error",
      "jsx-a11y/aria-props": "error",
      "jsx-a11y/aria-proptypes": "error",
      "jsx-a11y/aria-role": "error",
      "jsx-a11y/aria-unsupported-elements": "error",
      "jsx-a11y/heading-has-content": "error",
      "jsx-a11y/no-redundant-roles": "error",
      "jsx-a11y/role-has-required-aria-props": "error",
      "jsx-a11y/role-supports-aria-props": "error",
      "jsx-a11y/scope": "error",
      "jsx-a11y/tabindex-no-positive": "error",

      // ---- NOT YET CLEARED -- promote each as it is fixed ------------------
      // 7 findings. The four UI primitives generate no id, so nothing they
      // wrap can be labelled. Fix: one useId() in SettingContainer, threaded
      // to ToggleSwitch / Slider / Select / Input.
      "jsx-a11y/label-has-associated-control": "off",
      // 7 + 7 + 1 findings, largely the same sites: <div onClick> list rows in
      // Meetings, Notes and Home. Fix: make them real buttons.
      "jsx-a11y/no-static-element-interactions": "off",
      "jsx-a11y/click-events-have-key-events": "off",
      "jsx-a11y/interactive-supports-focus": "off",
      // 6 findings. Needs a per-case judgement -- autofocus is right in a
      // modal search field and wrong almost everywhere else.
      "jsx-a11y/no-autofocus": "off",
      // 3 findings, all the AudioPlayer <audio> element. Captions for the
      // user's own dictation recording is a product decision, not a lint fix.
      "jsx-a11y/media-has-caption": "off",
      // 14 findings. Each is a potential stale closure and each needs reading.
      // One is already known real: UpdateChecker.tsx captures `isChecking`
      // frozen at false, so a tray-initiated check bypasses the re-entry guard.
      "react-hooks/exhaustive-deps": "off",

      // ---- pre-existing, DOWNGRADED TO A TRACKED DEBT ----------------------
      // 21 findings across 10 files, all hardcoded English in JSX.
      //
      // This was "error", and it is being lowered deliberately -- flagging it
      // loudly because lowering a rule someone else set is not a decision to
      // make quietly. The reasoning: it has held `bun run lint` red for an
      // unknown length of time while the workflow that runs it sits disabled,
      // so in practice it enforced nothing and hid everything behind it. A gate
      // nobody can run is not a gate.
      //
      // As a warning it lets the ERROR band go green, which lets CI actually
      // block regressions -- including the twelve accessibility rules enforced
      // above. The debt does not get to grow: `i18n-literals-do-not-grow` in
      // tools/check_v1290.ps1 fails if the count exceeds 21, so this can only
      // ratchet down. Restore it to "error" in the commit that reaches zero.
      "i18next/no-literal-string": [
        "warn",
        {
          markupOnly: true, // Only check JSX content, not all strings
          ignoreAttribute: [
            "className",
            "style",
            "type",
            "id",
            "name",
            "key",
            "data-*",
            "aria-*",
          ], // Ignore common non-translatable attributes
        },
      ],
    },
  },
];
