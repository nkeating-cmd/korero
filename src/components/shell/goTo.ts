import { useNav } from "../../stores/navStore";
import type { ActivityTarget } from "../../stores/activityStore";

/** Open whatever an Activity item, a Today row or a search result points at. */
export const goTo = (target: ActivityTarget | undefined): void => {
  if (!target) return;
  const nav = useNav.getState();
  if (target.section === "meetings" && target.meetingId) {
    nav.openMeeting({ id: target.meetingId, tab: target.tab });
  } else if (target.section === "notes" && target.noteId) {
    nav.openNote(target.noteId);
  } else {
    nav.go(target.section);
  }
};
