import React from "react";
import { HelpSettings } from "./HelpSettings";
import { AboutSettings } from "../about/AboutSettings";

/** Kōrero 1.42: one "Help & about" page (the guide, then version, logs and folders). */
export const HelpAndAbout: React.FC = () => (
  <div className="flex flex-col gap-8">
    <HelpSettings />
    <AboutSettings />
  </div>
);
