import React from "react";
import { useTranslation } from "react-i18next";
import { Sparkles, Trash2 } from "lucide-react";
import { toast } from "sonner";
import type { LearnedPair } from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { useCorrections } from "./Corrections";
import { ToggleSwitch } from "./ToggleSwitch";

/**
 * How many times a replacement must be made before it is a rule: twice for a
 * phrase, three times for a single word (`LearnedPair::needed` in `learning.rs`).
 */
export const timesNeeded = (heard: string): number =>
  heard.includes(" ") ? 2 : 3;
/** A single word is offered after this many corrections (`SUGGEST_AT`). */
const OFFER_AT = 3;

/**
 * Kōrero 1.52 (B14): what Kōrero has learnt from the corrections made in
 * Dictation history, and the switch for it. Learnt pairs apply to dictation
 * only. "Use everywhere" turns one into a taught correction, which also
 * reaches meetings, notes and the AI clean-up; until then it is in no prompt.
 */
export const LearntCorrections: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const corrections = useCorrections();
  const enabled = getSetting("learning_enabled") !== false;
  const pairs: LearnedPair[] = getSetting("learned_pairs") ?? [];

  const save = (next: LearnedPair[]) => updateSetting("learned_pairs", next);
  const setOff = (heard: string, off: boolean) =>
    save(pairs.map((p) => (p.heard === heard ? { ...p, off } : p)));
  const forget = (heard: string) =>
    save(pairs.filter((p) => p.heard !== heard));
  const promote = (p: LearnedPair) => {
    const added = corrections.addMany([{ wrong: p.heard, right: p.meant }]);
    forget(p.heard);
    if (added > 0)
      toast.success(t("settings.debug.learning.promoted", { meant: p.meant }));
    else
      toast.message(
        t("settings.debug.learning.alreadyTaught", { heard: p.heard }),
      );
  };

  return (
    <div
      className="glass-card p-4 space-y-3"
      role="group"
      aria-label={t("settings.debug.learning.title")}
    >
      <div className="flex items-center gap-2">
        <Sparkles size={16} className="text-aurora-cyan" />
        <h3 className="text-sm font-semibold text-text">
          {t("settings.debug.learning.title")}
        </h3>
      </div>
      <p className="text-xs text-text-subtle leading-relaxed">
        {t("settings.debug.learning.description")}
      </p>
      <ToggleSwitch
        checked={enabled}
        onChange={(v) => updateSetting("learning_enabled", v)}
        isUpdating={isUpdating("learning_enabled")}
        label={t("settings.debug.learning.switch")}
        description={t("settings.debug.learning.switchDescription")}
        descriptionMode="tooltip"
        grouped={true}
      />
      {pairs.length === 0 ? (
        <p className="text-xs text-text-subtle">
          {t("settings.debug.learning.empty")}
        </p>
      ) : (
        <ul className="space-y-1">
          {pairs.map((p) => {
            const seen = p.seen ?? 0;
            const offer = p.suggest_only === true;
            const active = !p.off && !offer && seen >= timesNeeded(p.heard);
            const offered = !p.off && offer && seen >= OFFER_AT;
            const state = p.off
              ? t("settings.debug.learning.off")
              : active
                ? t("settings.debug.learning.active")
                : offered
                  ? t("settings.debug.learning.offered")
                  : t("settings.debug.learning.watching");
            return (
              <li
                key={p.heard}
                className="flex flex-wrap items-center gap-2 text-sm rounded-md px-2 py-1 hover:bg-white/5"
              >
                <span className="text-text-muted line-through">{p.heard}</span>
                <span className="text-text-subtle" aria-hidden="true">
                  →
                </span>
                <span className={p.off ? "text-text-muted" : "text-text"}>
                  {p.meant}
                </span>
                <span className="flex-1 text-xs text-text-subtle">
                  {state}
                  {" · "}
                  {t("settings.debug.learning.seen", { count: seen })}
                </span>
                {(active || offered) && (
                  <button
                    type="button"
                    title={t("settings.debug.learning.useEverywhereTitle")}
                    aria-label={t(
                      offered
                        ? "settings.debug.learning.alwaysFor"
                        : "settings.debug.learning.useEverywhereFor",
                      { heard: p.heard, meant: p.meant },
                    )}
                    onClick={() => promote(p)}
                    className="text-xs text-aurora-cyan hover:underline"
                  >
                    {t(
                      offered
                        ? "settings.debug.learning.always"
                        : "settings.debug.learning.useEverywhere",
                    )}
                  </button>
                )}
                <input
                  type="checkbox"
                  checked={!p.off}
                  onChange={(e) => setOff(p.heard, !e.target.checked)}
                  aria-label={t(
                    offer
                      ? "settings.debug.learning.keepCounting"
                      : "settings.debug.learning.use",
                    { heard: p.heard, meant: p.meant },
                  )}
                />
                <button
                  type="button"
                  aria-label={t("settings.debug.learning.forget", {
                    heard: p.heard,
                  })}
                  title={t("settings.debug.learning.forget", {
                    heard: p.heard,
                  })}
                  onClick={() => forget(p.heard)}
                  className="text-text-subtle hover:text-pill-urgent transition-colors"
                >
                  <Trash2 size={13} />
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
};
