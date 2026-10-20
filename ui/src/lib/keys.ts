// Converts browser keyboard events to the combo strings the backend parses
// (see app/src/hotkeys/combo.rs).

const MODIFIER_CODES = new Set([
  "ControlLeft", "ControlRight", "AltLeft", "AltRight", "ShiftLeft", "ShiftRight", "MetaLeft", "MetaRight",
]);

const NAMED: Record<string, string> = {
  Space: "Space", Enter: "Enter", Tab: "Tab", Backspace: "Backspace", Pause: "Pause", CapsLock: "CapsLock",
  PageUp: "PageUp", PageDown: "PageDown", End: "End", Home: "Home", Insert: "Insert", Delete: "Delete",
  ArrowLeft: "Left", ArrowUp: "Up", ArrowRight: "Right", ArrowDown: "Down", PrintScreen: "PrintScreen",
  ScrollLock: "ScrollLock", NumpadMultiply: "NumpadMultiply", NumpadAdd: "NumpadAdd",
  NumpadSubtract: "NumpadSubtract", NumpadDecimal: "NumpadDecimal", NumpadDivide: "NumpadDivide",
  Semicolon: "Semicolon", Equal: "Equal", Comma: "Comma", Minus: "Minus", Period: "Period", Slash: "Slash",
  Backquote: "Backquote", BracketLeft: "BracketLeft", Backslash: "Backslash", BracketRight: "BracketRight",
  Quote: "Quote",
};

function keyName(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);
  if (/^F\d{1,2}$/.test(code)) return code;
  if (/^Numpad\d$/.test(code)) return code;
  return NAMED[code] ?? null;
}

/** Returns the combo for a keydown event, or null while only modifiers are held. */
export function comboFromEvent(e: KeyboardEvent): string | null {
  if (MODIFIER_CODES.has(e.code)) return null;
  const key = keyName(e.code);
  if (!key) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Win");
  parts.push(key);
  return parts.join("+");
}
