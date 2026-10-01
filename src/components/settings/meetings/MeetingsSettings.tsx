/* eslint-disable i18next/no-literal-string */
import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ArrowLeft, Circle, Loader2, TriangleAlert, Upload, UsersRound, Waves } from "lucide-react";
import { useNav } from "../../../stores/navStore";
import { useRecorder } from "../../../stores/recorderStore";
import { useMeetingJobs } from "../../../stores/meetingJobsStore";
import { MeetingsLibrary } from "../../meetings/MeetingsLibrary";
import { MeetingView } from "../../meetings/MeetingView";
import { LiveMeetingView } from "../../meetings/LiveMeetingView";
import { ImportPanel, RecordingsPanel } from "../../meetings/ImportAndRecordings";
import { useMeetingsController, type MeetingsController } from "./useMeetingsController";

/**
 * Kōrero 1.42: the Meetings page (A2/A3 on the design canvas).
 *
 * A library on the left, the selected thing on the right: the meeting in
 * progress, an import, the recordings on disk, or a saved meeting. All the
 * logic lives in `useMeetingsController` (ported from the pre-1.42 page);
 * recording itself lives in `useRecorder`, so it carries on when you leave.
 *
 * Narrow windows show one pane at a time: the library, or the thing you
 * opened with a Back button to return.
 */

/** Below this page width, the library and the detail share one column. */
const NARROW_BELOW = 820;

const useElementWidth = (ref: React.RefObject<HTMLElement | null>) => {
  const [width, setWidth] = useState<number>(() => window.innerWidth);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setWidth(el.getBoundingClientRect().width);
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) setWidth(e.contentRect.width);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return width;
};

const LoadErrorBanner: React.FC<{ error: string }> = ({ error }) => (
  <div role="alert" className="kx-banner kx-banner-bad mx-8 mt-5">
    <TriangleAlert size={18} />
    <div className="flex-1 min-w-0">
      <p className="font-semibold">Couldn’t read your saved meetings</p>
      <p className="mt-1">
        Your meetings are still on disk — nothing has been deleted, and Kōrero will not overwrite
        the file while this message is showing. This is usually a file briefly locked by antivirus
        or cloud sync. Close and reopen Kōrero to try again.
      </p>
      <p className="kx-mono text-[12px] mt-2 break-all opacity-80">{error}</p>
    </div>
  </div>
);

const EmptyState: React.FC<{ c: MeetingsController }> = ({ c }) => {
  const starting = useRecorder((s) => s.starting);
  const start = async () => {
    const ok = await useRecorder.getState().start();
    if (ok) c.setView({ kind: "live" });
  };
  return (
    <div className="flex-1 flex items-center justify-center p-8">
      <div className="max-w-[440px] text-center flex flex-col items-center gap-3">
        <span className="kx-step kx-step-running" style={{ width: 44, height: 44, borderRadius: 22 }}>
          <UsersRound size={20} />
        </span>
        <h2 className="kx-title">Your meetings live here</h2>
        <p className="kx-meta">
          Record a call and Kōrero transcribes both sides on this computer, then writes the notes.
          Or bring in a recording you already have.
        </p>
        <div className="flex flex-wrap justify-center gap-2 mt-2">
          <button type="button" className="kx-btn kx-btn-primary" disabled={starting} onClick={() => void start()}>
            {starting ? <Loader2 size={14} className="animate-spin" /> : <Circle size={11} fill="currentColor" />}
            Record a meeting
          </button>
          <button type="button" className="kx-btn kx-btn-secondary" onClick={() => void c.pickImportFile()}>
            <Upload size={14} /> Import audio
          </button>
          <button type="button" className="kx-btn kx-btn-quiet" onClick={() => c.setView({ kind: "live" })}>
            <Waves size={14} /> Check my audio
          </button>
        </div>
      </div>
    </div>
  );
};

export const MeetingsSettings: React.FC = () => {
  const c = useMeetingsController();
  const rootRef = useRef<HTMLDivElement>(null);
  const width = useElementWidth(rootRef);
  const narrow = width < NARROW_BELOW;
  // Narrow layout only: true shows the library, false shows what you opened.
  const [listOpen, setListOpen] = useState(true);

  const recording = useRecorder((s) => s.recording);
  const stopping = useMeetingJobs((s) => s.stopping);
  const navSeq = useNav((s) => s.navSeq);

  // A request from elsewhere (search, Today, Activity, the companion) to open
  // something here. Read on mount and whenever a new request arrives.
  const firstFocus = useRef(true);
  useEffect(() => {
    const f = useNav.getState().takeMeetingFocus();
    const first = firstFocus.current;
    firstFocus.current = false;
    if (!f) {
      // Opening Meetings mid-recording lands on the meeting in progress.
      if (first && (useRecorder.getState().recording || useMeetingJobs.getState().stopping)) {
        c.setView({ kind: "live" });
        setListOpen(false);
      }
      return;
    }
    if (f.id === "live") c.setView({ kind: "live" });
    else if (f.id.startsWith("import:")) c.startImport(f.id.slice("import:".length));
    else c.focusMeeting(f.id, f.tab, f.query);
    setListOpen(false);
  }, [navSeq]);

  // Recording started (here, from the palette, the shortcut or Today): follow it.
  const wasRecording = useRef(recording);
  useEffect(() => {
    if (recording && !wasRecording.current) {
      c.setView({ kind: "live" });
      setListOpen(false);
    }
    wasRecording.current = recording;
  }, [recording]);

  // Anything other than a saved meeting is always a deliberate open.
  useEffect(() => {
    if (c.view.kind !== "meeting") setListOpen(false);
  }, [c.view.kind]);

  // In the narrow layout, choosing a meeting in the library opens it.
  const libraryC: MeetingsController = narrow
    ? {
        ...c,
        selectMeeting: (id: string) => {
          c.selectMeeting(id);
          setListOpen(false);
        },
        setView: (v) => {
          c.setView(v);
          setListOpen(false);
        },
      }
    : c;

  const back = narrow ? () => setListOpen(true) : undefined;
  const backBar = back ? (
    <div className="px-8 pt-4 shrink-0">
      <button type="button" className="kx-btn kx-btn-quiet kx-btn-sm -ml-2.5" onClick={back}>
        <ArrowLeft size={14} /> All meetings
      </button>
    </div>
  ) : null;

  const detail = (() => {
    if (c.view.kind === "live") {
      return (
        <>
          {backBar}
          <div className="flex-1 min-h-0 overflow-y-auto kx-cq">
            <LiveMeetingView />
          </div>
        </>
      );
    }
    if (c.view.kind === "import") {
      return (
        <>
          {backBar}
          <div className="flex-1 min-h-0 overflow-y-auto kx-cq">
            <ImportPanel c={c} />
          </div>
        </>
      );
    }
    if (c.view.kind === "recordings") {
      return (
        <>
          {backBar}
          <div className="flex-1 min-h-0 overflow-y-auto kx-cq">
            <RecordingsPanel c={c} />
          </div>
        </>
      );
    }
    if (!c.storeReady && !c.storeLoadError) {
      return (
        <div className="flex-1 flex items-center justify-center kx-meta gap-2" aria-live="polite">
          <Loader2 size={14} className="animate-spin" /> Loading your meetings…
        </div>
      );
    }
    if (c.active) return <MeetingView c={c} onBack={back} />;
    if (c.storeLoadError) return null;
    return <EmptyState c={c} />;
  })();

  const showLibrary = !narrow || listOpen;
  const showDetail = !narrow || !listOpen;

  return (
    <div ref={rootRef} className="flex-1 min-h-0 flex">
      {showLibrary && <MeetingsLibrary c={libraryC} wide={narrow} />}
      {showDetail && (
        <div className="flex-1 min-w-0 min-h-0 flex flex-col">
          {c.storeLoadError && <LoadErrorBanner error={c.storeLoadError} />}
          {detail}
        </div>
      )}
      {/* Keep "recording" in a live region so screen readers hear it change. */}
      <span className="kx-sr-only" aria-live="polite">
        {recording ? "Recording a meeting" : stopping ? "Saving the meeting" : ""}
      </span>
    </div>
  );
};
