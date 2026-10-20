import { useEffect } from "react";
import { PageHeader } from "../../components/ui";
import { useSettingsStore } from "../../store/settingsStore";
import { AdminSection } from "./AdminSection";
import { EncoderSection } from "./EncoderSection";
import { GeneralSection } from "./GeneralSection";
import { OutputSection } from "./OutputSection";
import { PresetsSection } from "./PresetsSection";
import { ReplaySection } from "./ReplaySection";
import { SharingSection } from "./SharingSection";
import { ViewerSection } from "./ViewerSection";

export function SettingsPage() {
  const settings = useSettingsStore((s) => s.settings);
  const flush = useSettingsStore((s) => s.flush);

  // Save pending text edits when leaving the page.
  useEffect(() => () => void flush(), [flush]);

  if (!settings) return <div className="p-5 text-sm text-zinc-500">Loading settings…</div>;

  return (
    <div className="flex flex-col gap-4 p-5">
      <PageHeader title="Settings" subtitle="Changes are saved automatically." />
      <div className="grid items-start gap-4 @5xl:grid-cols-2">
        <div className="flex min-w-0 flex-col gap-4">
          <EncoderSection enc={settings.encoder} />
          <ReplaySection settings={settings} />
          <PresetsSection settings={settings} />
          <ViewerSection settings={settings} />
        </div>
        <div className="flex min-w-0 flex-col gap-4">
          <OutputSection settings={settings} />
          <GeneralSection settings={settings} />
          <SharingSection settings={settings} />
          <AdminSection />
        </div>
      </div>
    </div>
  );
}
