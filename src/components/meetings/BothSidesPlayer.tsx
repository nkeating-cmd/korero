/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useRef, useState } from "react";
import { Pause, Play } from "lucide-react";
import { fmtClock } from "./model";

/**
 * Kōrero 1.46: play a call's two recordings together. The microphone and the call's
 * audio are saved as separate files that start within a few milliseconds of each
 * other; this keeps two audio elements in step behind one set of controls, so the
 * conversation can be heard as it happened. The second element follows the first:
 * it is nudged back whenever it drifts by more than a quarter of a second.
 */
export const BothSidesPlayer: React.FC<{ a: string; b: string }> = ({ a, b }) => {
  const first = useRef<HTMLAudioElement | null>(null);
  const second = useRef<HTMLAudioElement | null>(null);
  const [playing, setPlaying] = useState(false);
  const [at, setAt] = useState(0);
  const [length, setLength] = useState(0);
  const [failed, setFailed] = useState(false);

  // A different meeting: stop and start from the beginning.
  useEffect(() => {
    setPlaying(false);
    setAt(0);
    setLength(0);
    setFailed(false);
  }, [a, b]);

  const follow = () => {
    const x = first.current;
    const y = second.current;
    if (!x || !y) return;
    setAt(x.currentTime);
    if (Math.abs(y.currentTime - x.currentTime) > 0.25 && x.currentTime < (y.duration || Infinity)) {
      y.currentTime = x.currentTime;
    }
  };
  const measure = () => {
    const d = Math.max(first.current?.duration || 0, second.current?.duration || 0);
    if (Number.isFinite(d)) setLength(d);
  };
  const toggle = async () => {
    const x = first.current;
    const y = second.current;
    if (!x || !y) return;
    if (playing) {
      x.pause();
      y.pause();
      setPlaying(false);
      return;
    }
    try {
      y.currentTime = x.currentTime;
      await Promise.all([x.play(), y.play()]);
      setPlaying(true);
    } catch {
      x.pause();
      y.pause();
      setFailed(true);
    }
  };
  const seek = (t: number) => {
    const x = first.current;
    const y = second.current;
    if (!x || !y) return;
    x.currentTime = t;
    y.currentTime = Math.min(t, y.duration || t);
    setAt(t);
  };
  const stop = () => {
    first.current?.pause();
    second.current?.pause();
    setPlaying(false);
  };

  return (
    <div className="flex items-center gap-3">
      <audio ref={first} src={a} preload="metadata" onTimeUpdate={follow} onLoadedMetadata={measure} onEnded={stop} onError={() => setFailed(true)} />
      <audio ref={second} src={b} preload="metadata" onLoadedMetadata={measure} onError={() => setFailed(true)} />
      <button
        type="button"
        className="kx-btn kx-btn-secondary kx-btn-sm"
        onClick={() => void toggle()}
        disabled={failed}
        aria-label={playing ? "Pause both sides" : "Play both sides together"}
      >
        {playing ? <Pause size={14} /> : <Play size={14} />}
      </button>
      <span className="kx-mono kx-meta w-12 text-right">{fmtClock(at)}</span>
      <input
        type="range"
        className="flex-1"
        min={0}
        max={length || 0}
        step={1}
        value={Math.min(at, length || 0)}
        onChange={(e) => seek(Number(e.target.value))}
        disabled={failed || length === 0}
        aria-label="Position in the call"
      />
      <span className="kx-mono kx-meta w-12">{fmtClock(length)}</span>
      {failed && <span className="kx-meta" role="alert">One of the recordings could not be played.</span>}
    </div>
  );
};
