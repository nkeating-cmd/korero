/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useRef } from "react";
import { Check, TriangleAlert, X, CircleAlert, Square } from "lucide-react";
import { useNav } from "../../stores/navStore";
import { useActivity, type ActivityItem } from "../../stores/activityStore";
import { useRecorder } from "../../stores/recorderStore";
import { useRunning, fmtSince } from "./useRunning";
import { goTo } from "./goTo";

/**
 * Kōrero 1.42: the Activity panel. Running work comes live from the stores
 * that own it; finished, failed and needs-a-look items come from the session
 * log. Non-modal: the page beside it stays usable. Esc closes it.
 */

const timeOf = (at: number) =>
  new Date(at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });

const Outcome: React.FC<{ item: ActivityItem }> = ({ item }) => {
  const icon =
    item.status === "done" ? (
      <Check size={16} className="text-[var(--kx-ok)]" aria-label="Done" />
    ) : item.status === "attention" ? (
      <TriangleAlert size={16} className="text-[var(--kx-warn)]" aria-label="Needs a look" />
    ) : (
      <CircleAlert size={16} className="text-[var(--kx-alert)]" aria-label="Failed" />
    );
  const body = (
    <>
      <span className="mt-0.5 shrink-0">{icon}</span>
      <span className="flex-1 min-w-0">
        <span className="block text-[13px] text-white">{item.title}</span>
        {item.detail && (
          <span className="block kx-meta mt-0.5 line-clamp-3">{item.detail}</span>
        )}
      </span>
      <span className="kx-meta kx-mono shrink-0">{timeOf(item.at)}</span>
    </>
  );
  return item.target ? (
    <button type="button" className="kx-row px-2 py-2.5 items-start rounded-[8px]" onClick={() => goTo(item.target)}>
      {body}
    </button>
  ) : (
    <div className="flex gap-3.5 items-start px-2 py-2.5">{body}</div>
  );
};

export const ActivityPanel: React.FC = () => {
  const open = useNav((s) => s.activityOpen);
  const setOpen = useNav((s) => s.setActivityOpen);
  const items = useActivity((s) => s.items);
  const running = useRunning();
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    useActivity.getState().markSeen();
    closeRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, setOpen]);

  // Mark new outcomes seen while the panel is showing them.
  useEffect(() => {
    if (open) useActivity.getState().markSeen();
  }, [open, items.length]);

  if (!open) return null;

  const attention = items.filter((i) => i.status !== "done");
  const done = items.filter((i) => i.status === "done");

  return (
    <aside
      aria-labelledby="kx-activity-h"
      className="kx-slide-in w-[372px] shrink-0 flex flex-col min-h-0 border-l border-[var(--kx-hairline)] bg-[var(--kx-surface)]"
      style={{ boxShadow: "-12px 0 32px rgba(0,0,0,.35)" }}
    >
      <div className="flex items-center justify-between px-4 pt-4 pb-2">
        <h2 id="kx-activity-h" className="text-[16px] font-semibold text-white">
          Activity
        </h2>
        <button
          ref={closeRef}
          type="button"
          aria-label="Close Activity"
          onClick={() => setOpen(false)}
          className="kx-btn kx-btn-ghost kx-btn-icon kx-btn-sm"
        >
          <X size={16} />
        </button>
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto px-4 pb-4">
        <h3 className="kx-overline pt-1 pb-2">Running</h3>
        {running.length === 0 ? (
          <p className="kx-meta pb-2">Nothing running. Work you start here keeps going when you leave a page.</p>
        ) : (
          <div className="flex flex-col gap-2">
            {running.map((r) => (
              <div key={r.id} className="kx-raised p-3">
                <div className="flex items-baseline justify-between gap-2">
                  <span className="text-[13.5px] font-semibold text-white">{r.title}</span>
                  <span className="kx-meta kx-mono">
                    {r.progress !== null ? `${r.progress}%` : fmtSince(r.startedAt)}
                  </span>
                </div>
                <div className="kx-meta mt-0.5">{r.detail}</div>
                {r.kind !== "recording" && (
                  <div className={`kx-progress mt-2.5 ${r.progress === null ? "kx-progress-indeterminate" : ""}`}>
                    <span style={r.progress !== null ? { width: `${r.progress}%` } : undefined} />
                  </div>
                )}
                <div className="flex gap-2 mt-2.5">
                  <button type="button" className="kx-btn kx-btn-secondary kx-btn-sm" onClick={() => goTo(r.target)}>
                    {r.kind === "notes" ? "Watch it write" : "Open"}
                  </button>
                  {r.kind === "recording" && (
                    <button type="button" className="kx-btn kx-btn-danger kx-btn-sm" onClick={() => useRecorder.getState().stop()}>
                      <Square size={10} fill="currentColor" /> Stop
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}

        {attention.length > 0 && (
          <>
            <h3 className="kx-overline pt-5 pb-1">Needs a look</h3>
            <div className="kx-divide">
              {attention.map((i) => (
                <Outcome key={i.id} item={i} />
              ))}
            </div>
          </>
        )}

        <div className="flex items-center justify-between pt-5 pb-1">
          <h3 className="kx-overline">Done this session</h3>
          {done.length > 0 && (
            <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm" onClick={() => useActivity.getState().clearDone()}>
              Clear
            </button>
          )}
        </div>
        {done.length === 0 ? (
          <p className="kx-meta">Finished work appears here.</p>
        ) : (
          <div className="kx-divide">
            {done.map((i) => (
              <Outcome key={i.id} item={i} />
            ))}
          </div>
        )}
      </div>

      <p className="kx-meta px-4 py-3 border-t border-[var(--kx-hairline)]">
        Work keeps going if you switch pages or close the window to the tray.
      </p>
    </aside>
  );
};
