import React from "react";
import { Headphones, UsersRound } from "lucide-react";
import { useRecorder } from "../../stores/recorderStore";
import { useMeetingJobs } from "../../stores/meetingJobsStore";
import type { MeetingMode } from "./model";

/**
 * Kōrero 1.45 (B3): what kind of meeting this is, chosen before Record and remembered.
 * "On a call" records your mic and the call's audio. "In the room" records the mic only
 * (there is no call to capture) and, after Stop, rebuilds the transcript by speaker.
 */
export const MeetingModePicker: React.FC<{ className?: string }> = ({ className }) => {
  const mode = useRecorder((s) => s.mode);
  const recording = useRecorder((s) => s.recording);
  // Review L6: no change of type while the last meeting is still being saved.
  const stopping = useMeetingJobs((s) => s.stopping !== null);
  const locked = recording || stopping;
  const choose = (value: MeetingMode) => useRecorder.getState().setMode(value);
  // Review L7: a radio group is one tab stop; the arrow keys move and choose.
  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (locked || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) return;
    e.preventDefault();
    const next: MeetingMode = mode === "call" ? "room" : "call";
    choose(next);
    const el = e.currentTarget.querySelector<HTMLButtonElement>(`[data-mode="${next}"]`);
    el?.focus();
  };
  const option = (value: MeetingMode, label: string, hint: string, icon: React.ReactNode) => (
    <button
      type="button"
      role="radio"
      data-mode={value}
      aria-checked={mode === value}
      tabIndex={mode === value ? 0 : -1}
      title={hint}
      disabled={locked}
      onClick={() => choose(value)}
      className="kx-choice-option"
    >
      {icon}
      {label}
    </button>
  );
  return (
    <div role="radiogroup" aria-label="Meeting type" onKeyDown={onKeyDown} className={`kx-choice ${className ?? ""}`}>
      {option("call", "On a call", "Records your microphone and the call's audio as two sides", <Headphones size={13} />)}
      {option(
        "room",
        "In the room",
        "Everyone is in the room: records your microphone only, then sorts the transcript by speaker",
        <UsersRound size={13} />,
      )}
    </div>
  );
};
