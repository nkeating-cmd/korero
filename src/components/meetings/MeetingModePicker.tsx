import React from "react";
import { Headphones, UsersRound } from "lucide-react";
import { useRecorder } from "../../stores/recorderStore";
import type { MeetingMode } from "./model";

/**
 * Kōrero 1.45 (B3): what kind of meeting this is, chosen before Record and remembered.
 * "On a call" records your mic and the call's audio. "In the room" records the mic only
 * (there is no call to capture) and, after Stop, rebuilds the transcript by speaker.
 */
export const MeetingModePicker: React.FC<{ className?: string }> = ({ className }) => {
  const mode = useRecorder((s) => s.mode);
  const recording = useRecorder((s) => s.recording);
  const option = (value: MeetingMode, label: string, hint: string, icon: React.ReactNode) => (
    <button
      type="button"
      role="radio"
      aria-checked={mode === value}
      title={hint}
      disabled={recording}
      onClick={() => useRecorder.getState().setMode(value)}
      className="kx-choice-option"
    >
      {icon}
      {label}
    </button>
  );
  return (
    <div role="radiogroup" aria-label="Meeting type" className={`kx-choice ${className ?? ""}`}>
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
