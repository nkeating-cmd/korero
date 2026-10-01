// Kōrero 1.42: small app-wide facts that more than one page shows.
import { create } from "zustand";

interface AppStatus {
  /** Set when the startup update check found a newer release. */
  updateAvailable: { version: string; url: string } | null;
  setUpdateAvailable: (u: { version: string; url: string } | null) => void;
}

export const useAppStatus = create<AppStatus>((set) => ({
  updateAvailable: null,
  setUpdateAvailable: (updateAvailable) => set({ updateAvailable }),
}));
