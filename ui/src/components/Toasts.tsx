import { useNoticeStore, type NoticeKind } from "../store/noticeStore";
import { IconX } from "./icons";
import { cx } from "./ui";

const KIND_STYLE: Record<NoticeKind, string> = {
  info: "border-zinc-700",
  success: "border-emerald-700/70",
  error: "border-red-800/80",
};

const DOT: Record<NoticeKind, string> = {
  info: "bg-sky-400",
  success: "bg-emerald-400",
  error: "bg-red-400",
};

export function Toasts() {
  const notices = useNoticeStore((s) => s.notices);
  const dismiss = useNoticeStore((s) => s.dismiss);
  if (notices.length === 0) return null;
  return (
    <div className="pointer-events-none fixed right-4 bottom-4 z-50 flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2">
      {notices.map((n) => (
        <div
          key={n.id}
          role="status"
          className={cx(
            "pointer-events-auto flex gap-3 rounded-lg border bg-zinc-900/95 px-3 py-2.5 shadow-lg shadow-black/40 backdrop-blur",
            KIND_STYLE[n.kind],
          )}
        >
          <span className={cx("mt-1.5 h-2 w-2 shrink-0 rounded-full", DOT[n.kind])} />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-medium text-zinc-100">{n.title}</div>
            {n.body && <div className="mt-0.5 text-xs break-all text-zinc-400 select-text">{n.body}</div>}
            {n.action && (
              <button
                onClick={() => {
                  n.action?.run();
                  dismiss(n.id);
                }}
                className="mt-1 text-xs font-medium text-sky-400 hover:text-sky-300"
              >
                {n.action.label}
              </button>
            )}
          </div>
          <button onClick={() => dismiss(n.id)} className="h-5 shrink-0 text-zinc-500 hover:text-zinc-300" aria-label="Dismiss">
            <IconX size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
