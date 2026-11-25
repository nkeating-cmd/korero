import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useSettings } from "../../hooks/useSettings";
import { normaliseAddress } from "../../lib/addressLearning";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { SettingContainer } from "../ui/SettingContainer";

interface KnownAddressesProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * Kōrero 1.51: "Your email addresses". A dictated address that sounds like
 * one of these is written as it (`known_addresses.rs`, dictation only).
 */
export const KnownAddresses: React.FC<KnownAddressesProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const [draft, setDraft] = useState("");
    const list = getSetting("known_addresses") || [];
    const busy = isUpdating("known_addresses");

    const add = () => {
      const a = normaliseAddress(draft);
      if (!a) {
        toast.error(t("settings.debug.knownAddresses.invalid"));
        return;
      }
      if (list.includes(a)) {
        toast.error(
          t("settings.debug.knownAddresses.duplicate", { address: a }),
        );
        return;
      }
      updateSetting("known_addresses", [...list, a]);
      setDraft("");
    };

    return (
      <>
        <SettingContainer
          title={t("settings.debug.knownAddresses.title")}
          description={t("settings.debug.knownAddresses.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
        >
          <div className="flex items-center gap-2">
            <Input
              type="email"
              className="max-w-56"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  add();
                }
              }}
              placeholder={t("settings.debug.knownAddresses.placeholder")}
              aria-label={t("settings.debug.knownAddresses.title")}
              variant="compact"
              disabled={busy}
            />
            <Button
              onClick={add}
              disabled={!draft.trim() || busy}
              variant="primary"
              size="md"
            >
              {t("settings.debug.knownAddresses.add")}
            </Button>
          </div>
        </SettingContainer>
        {list.length > 0 && (
          <div
            className={`px-4 p-2 ${grouped ? "" : "rounded-lg border border-mid-gray/20"} flex flex-wrap gap-1`}
          >
            {list.map((address) => (
              <Button
                key={address}
                onClick={() =>
                  updateSetting(
                    "known_addresses",
                    list.filter((a) => a !== address),
                  )
                }
                disabled={busy}
                variant="secondary"
                size="sm"
                className="inline-flex items-center gap-1 cursor-pointer"
                aria-label={t("settings.debug.knownAddresses.remove", {
                  address,
                })}
              >
                <span>{address}</span>
                <svg
                  aria-hidden="true"
                  className="w-3 h-3"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M6 18L18 6M6 6l12 12"
                  />
                </svg>
              </Button>
            ))}
          </div>
        )}
      </>
    );
  },
);
