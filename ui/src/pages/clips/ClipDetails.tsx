import { useState } from "react";
import { Button } from "../../components/ui";
import { api, type ClipInfo, type ClipDetails as Details } from "../../lib/ipc";
import { attempt, notify } from "../../store/noticeStore";

export function ClipDetails({ clip, playhead, onSeek, onChanged }: { clip: ClipInfo; playhead: number; onSeek: (t: number) => void; onChanged: () => void }) {
  const [details, setDetails] = useState<Details>(clip.details ?? { favorite: false, tags: [], notes: "", bookmarks: [] });
  const [tags, setTags] = useState(details.tags.join(", "));
  const [label, setLabel] = useState("");
  const [saving, setSaving] = useState(false);
  const field = "rounded border border-zinc-600 bg-zinc-950 p-2 text-zinc-100";
  const save = async () => {
    setSaving(true);
    try { await attempt("Could not save clip details", async () => {
      const next = { ...details, tags: tags.split(",").map((t) => t.trim()).filter(Boolean) };
      await api.setClipDetails(clip.path, next); setDetails(next); onChanged(); notify({ kind: "success", title: "Clip details saved" });
    }); } finally { setSaving(false); }
  };
  return <details className="border-t border-zinc-700 p-4 text-sm text-zinc-300"><summary className="cursor-pointer">Tags, favorites & named bookmarks</summary>
    <fieldset disabled={saving} className="mt-3 space-y-3">
      <label className="block"><input type="checkbox" checked={details.favorite} onChange={(e) => setDetails({ ...details, favorite: e.target.checked })} /> Favorite — protect from automatic cleanup</label>
      <label className="block">Tags (comma separated) <input aria-label="Clip tags" className={`${field} w-full`} value={tags} maxLength={2500} onChange={(e) => setTags(e.target.value)} /></label>
      <label className="block">Notes <textarea aria-label="Clip notes" className={`${field} w-full`} value={details.notes} maxLength={4000} onChange={(e) => setDetails({ ...details, notes: e.target.value })} /></label>
      <div className="flex flex-wrap gap-2"><input aria-label="Bookmark label" className={field} placeholder="Bookmark label" maxLength={120} value={label} onChange={(e) => setLabel(e.target.value)} />
        <Button size="sm" disabled={!label.trim() || details.bookmarks.length >= 200} onClick={() => { setDetails({ ...details, bookmarks: [...details.bookmarks, { time: playhead, label: label.trim() }].sort((a,b) => a.time-b.time) }); setLabel(""); }}>Mark {playhead.toFixed(2)} s</Button>
      </div>
      <ul className="max-h-40 space-y-1 overflow-y-auto">{details.bookmarks.map((b,i) => <li key={i} className="flex gap-2"><button className="flex-1 text-left text-sky-300" onClick={() => onSeek(b.time)}>{b.time.toFixed(2)} s · {b.label}</button><Button size="sm" onClick={() => setDetails({ ...details, bookmarks: details.bookmarks.filter((_,k) => k !== i) })}>Remove</Button></li>)}</ul>
      <Button size="sm" onClick={() => void save()}>{saving ? "Saving…" : "Save details"}</Button>
    </fieldset>
  </details>;
}
