//! Global hotkeys that keep working while a game has focus.
//!
//! A dedicated OS thread owns a Win32 message queue. Each binding is first
//! registered with `RegisterHotKey`, which costs nothing per keystroke. If that
//! fails (usually because another app such as ShadowPlay already owns the combo),
//! or if the user asks for it, the binding falls back to a low-level keyboard hook
//! (`WH_KEYBOARD_LL`). The hook is installed only while at least one binding
//! needs it, and its callback does a table lookup and posts a message, nothing
//! more, so it adds no measurable input latency.

mod combo;
mod sys;

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use lanes_core::Timestamp;
use serde::{Deserialize, Serialize};

pub use combo::Combo;

/// Which mechanism the user wants for a binding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// `RegisterHotKey`, falling back to the keyboard hook if the combo is taken.
    #[default]
    Auto,
    /// Always use the low-level keyboard hook. Unlike `RegisterHotKey`, the
    /// hook does not swallow the key, so the game still sees it.
    Hook,
}

/// The mechanism a binding actually ended up using.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Registered,
    Hook,
}

/// A hotkey press, stamped on the shared QPC clock at the moment it was seen.
#[derive(Clone, Debug)]
pub struct HotkeyEvent {
    pub id: String,
    pub combo: Combo,
    pub mode: Mode,
    pub at: Timestamp,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum HotkeyError {
    #[error("{0} is already used by \"{1}\"")]
    Conflict(Combo, String),
    #[error("could not install the keyboard hook: {0}")]
    Hook(String),
    #[error("the hotkey thread is not running")]
    ThreadGone,
}

enum Command {
    Bind { id: String, combo: Combo, backend: Backend, reply: mpsc::Sender<Result<Mode, HotkeyError>> },
    Unbind { id: String },
    Shutdown,
}

/// Handle to the hotkey thread. Dropping it unregisters everything and joins the thread.
pub struct HotkeyService {
    thread_id: u32,
    commands: mpsc::Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl HotkeyService {
    /// Spawns the hotkey thread. `on_event` runs on that thread for every press,
    /// so it should hand work off quickly (for example by emitting an event).
    pub fn start(on_event: impl Fn(HotkeyEvent) + Send + 'static) -> std::io::Result<Self> {
        let (commands, command_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new().name("lanes-hotkeys".into()).spawn(move || {
            let thread_id = sys::current_thread_id_with_queue();
            let _ = ready_tx.send(thread_id);
            Worker::new(thread_id, Box::new(on_event)).run(&command_rx);
        })?;
        let thread_id = ready_rx
            .recv()
            .map_err(|_| std::io::Error::other("hotkey thread exited during startup"))?;
        Ok(Self { thread_id, commands, thread: Some(thread) })
    }

    /// Binds `combo` to `id`, replacing any previous combo for that id.
    pub fn bind(&self, id: &str, combo: Combo, backend: Backend) -> Result<Mode, HotkeyError> {
        let (reply, reply_rx) = mpsc::channel();
        self.send(Command::Bind { id: id.to_owned(), combo, backend, reply })?;
        reply_rx.recv().map_err(|_| HotkeyError::ThreadGone)?
    }

    /// Removes the binding for `id`, if any.
    pub fn unbind(&self, id: &str) {
        let _ = self.send(Command::Unbind { id: id.to_owned() });
    }

    fn send(&self, command: Command) -> Result<(), HotkeyError> {
        self.commands.send(command).map_err(|_| HotkeyError::ThreadGone)?;
        sys::post(self.thread_id, sys::WM_APP_COMMAND, 0, 0).then_some(()).ok_or(HotkeyError::ThreadGone)
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        let _ = self.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Binding {
    id: String,
    combo: Combo,
    native_id: i32,
    mode: Mode,
}

/// State owned by the hotkey thread.
struct Worker {
    thread_id: u32,
    bindings: Vec<Binding>,
    hook: Option<sys::Hook>,
    on_event: Box<dyn Fn(HotkeyEvent)>,
}

impl Worker {
    fn new(thread_id: u32, on_event: Box<dyn Fn(HotkeyEvent)>) -> Self {
        Self { thread_id, bindings: Vec::new(), hook: None, on_event }
    }

    fn run(mut self, commands: &mpsc::Receiver<Command>) {
        while let Some(msg) = sys::next_message() {
            match msg {
                sys::Message::Hotkey(native_id) => self.fire(native_id, Timestamp::now()),
                sys::Message::HookFired(native_id, at) => self.fire(native_id, at),
                sys::Message::Command => {
                    for command in commands.try_iter() {
                        match command {
                            Command::Bind { id, combo, backend, reply } => {
                                let _ = reply.send(self.bind(id, combo, backend));
                            }
                            Command::Unbind { id } => self.unbind(&id),
                            Command::Shutdown => return self.shutdown(),
                        }
                    }
                }
                sys::Message::Other => {}
            }
        }
        self.shutdown();
    }

    fn fire(&self, native_id: i32, at: Timestamp) {        let Some(b) = self.bindings.iter().find(|b| b.native_id == native_id) else { return };
        let event = HotkeyEvent { id: b.id.clone(), combo: b.combo, mode: b.mode, at };
        // A panicking callback must not take global hotkeys down with it.
        if catch_unwind(AssertUnwindSafe(|| (self.on_event)(event))).is_err() {
            log::error!("hotkey handler for \"{}\" panicked", b.id);
        }
    }

    fn bind(&mut self, id: String, combo: Combo, backend: Backend) -> Result<Mode, HotkeyError> {
        if let Some(other) = self.bindings.iter().find(|b| b.combo == combo && b.id != id) {
            return Err(HotkeyError::Conflict(combo, other.id.clone()));
        }
        self.unbind(&id);

        // RegisterHotKey ids must be in 0x0000..=0xBFFF; reuse the smallest free one.
        let native_id = (1..0xC000).find(|n| self.bindings.iter().all(|b| b.native_id != *n)).unwrap_or(0xBFFF);

        let mode = match backend {
            Backend::Auto => match sys::register_hotkey(native_id, &combo) {
                Ok(()) => Mode::Registered,
                Err(e) => {
                    log::warn!("{combo} could not be registered ({e}); falling back to the keyboard hook");
                    Mode::Hook
                }
            },
            Backend::Hook => Mode::Hook,
        };
        self.bindings.push(Binding { id, combo, native_id, mode });
        // Only installing the hook can fail, so the new binding is a hook binding.
        if let Err(e) = self.sync_hook() {
            self.bindings.pop();
            let _ = self.sync_hook();
            return Err(e);
        }
        Ok(mode)
    }

    fn unbind(&mut self, id: &str) {
        let Some(pos) = self.bindings.iter().position(|b| b.id == id) else { return };
        let b = self.bindings.remove(pos);
        if b.mode == Mode::Registered {
            sys::unregister_hotkey(b.native_id);
        }
        // Removing a hook binding can only shrink the table, which cannot fail.
        let _ = self.sync_hook();
    }

    /// Installs the hook while any binding needs it, removes it otherwise, and
    /// refreshes the table the hook callback reads.
    fn sync_hook(&mut self) -> Result<(), HotkeyError> {
        let entries: Vec<HookEntry> = self
            .bindings
            .iter()
            .filter(|b| b.mode == Mode::Hook)
            .map(|b| HookEntry { native_id: b.native_id, combo: b.combo, down: false })
            .collect();
        let needed = !entries.is_empty();
        let installing = needed && self.hook.is_none();
        HOOK_TABLE.with_borrow_mut(|t| {
            t.thread_id = self.thread_id;
            t.entries = entries;
            if installing {
                t.held = ModifierState::snapshot(sys::key_down);
            }
        });
        match (needed, self.hook.is_some()) {
            (true, false) => {
                self.hook = Some(sys::Hook::install(keyboard_hook).map_err(|e| HotkeyError::Hook(e.to_string()))?);
                log::info!("keyboard hook installed");
            }
            (false, true) => {
                self.hook = None;
                log::info!("keyboard hook removed");
            }
            _ => {}
        }
        Ok(())
    }

    fn shutdown(&mut self) {
        for b in self.bindings.drain(..) {
            if b.mode == Mode::Registered {
                sys::unregister_hotkey(b.native_id);
            }
        }
        self.hook = None;
    }
}

struct HookEntry {
    native_id: i32,
    combo: Combo,
    /// Set while the key is held, to ignore auto-repeat.
    down: bool,
}

#[derive(Default)]
struct HookTable {
    thread_id: u32,
    entries: Vec<HookEntry>,
    held: ModifierState,
}

/// Which modifier keys are held, tracked from the hook's own event stream.
///
/// `GetAsyncKeyState` lags behind inside a low-level hook, so we follow the
/// left/right-specific key events the hook receives instead. If a key-up is
/// missed (for example on the secure desktop after Ctrl+Alt+Del), the state
/// heals on the next press and release of that key.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct ModifierState(u8);

impl ModifierState {
    /// Left/right virtual-key codes, in bit order: Ctrl, Alt, Shift, Win pairs.
    const KEYS: [u16; 8] = [0xA2, 0xA3, 0xA4, 0xA5, 0xA0, 0xA1, 0x5B, 0x5C];

    fn snapshot(key_down: impl Fn(u16) -> bool) -> Self {
        let mut state = Self::default();
        for vk in Self::KEYS {
            state.update(vk, key_down(vk));
        }
        state
    }

    /// Records a key transition. Returns true if `vk` is a modifier.
    fn update(&mut self, vk: u16, pressed: bool) -> bool {
        let Some(bit) = Self::KEYS.iter().position(|&k| k == vk) else { return false };
        if pressed {
            self.0 |= 1 << bit;
        } else {
            self.0 &= !(1 << bit);
        }
        true
    }

    /// True if exactly the combo's modifiers are held. Extra modifiers do not
    /// match, so Ctrl+Shift+F9 does not fire on Ctrl+Alt+Shift+F9.
    fn matches(self, combo: &Combo) -> bool {
        let pair = |shift: u8| self.0 & (0b11 << shift) != 0;
        pair(0) == combo.ctrl && pair(2) == combo.alt && pair(4) == combo.shift && pair(6) == combo.win
    }
}

thread_local! {
    // Low-level hooks are called on the thread that installed them, so a
    // thread-local table needs no locking.
    static HOOK_TABLE: RefCell<HookTable> = RefCell::default();
}

/// Called for every key event system-wide while the hook is installed. It must
/// stay tiny: no allocation, no logging, no locks.
fn keyboard_hook(event: sys::KeyEvent) {
    HOOK_TABLE.with(|cell| {
        let Ok(mut table) = cell.try_borrow_mut() else { return };
        if table.held.update(event.vk, event.pressed) {
            return;
        }
        let (thread_id, held) = (table.thread_id, table.held);
        for e in table.entries.iter_mut().filter(|e| e.combo.vk == event.vk) {
            if !event.pressed {
                e.down = false;
            } else if !e.down && held.matches(&e.combo) {
                e.down = true;
                sys::post_hook_fired(thread_id, e.native_id, Timestamp::now());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const LCTRL: u16 = 0xA2;
    const RCTRL: u16 = 0xA3;
    const LALT: u16 = 0xA4;
    const LSHIFT: u16 = 0xA0;
    const LWIN: u16 = 0x5B;

    fn combo(s: &str) -> Combo {
        s.parse().unwrap()
    }

    #[test]
    fn tracks_presses_and_releases() {
        let mut m = ModifierState::default();
        assert!(m.update(LCTRL, true));
        assert!(m.update(LSHIFT, true));
        assert!(m.matches(&combo("Ctrl+Shift+F9")));
        m.update(LSHIFT, false);
        assert!(m.matches(&combo("Ctrl+F9")));
        assert!(!m.matches(&combo("Ctrl+Shift+F9")));
    }

    #[test]
    fn extra_modifiers_do_not_match() {
        let mut m = ModifierState::default();
        for vk in [LCTRL, LALT, LSHIFT] {
            m.update(vk, true);
        }
        assert!(!m.matches(&combo("Ctrl+Shift+F9")));
        assert!(m.matches(&combo("Ctrl+Alt+Shift+F9")));
    }

    #[test]
    fn either_side_counts_until_both_are_released() {
        let mut m = ModifierState::default();
        m.update(LCTRL, true);
        m.update(RCTRL, true);
        m.update(LCTRL, false);
        assert!(m.matches(&combo("Ctrl+K")));
        m.update(RCTRL, false);
        assert!(m.matches(&combo("K")));
    }

    #[test]
    fn ignores_non_modifiers() {
        let mut m = ModifierState::default();
        assert!(!m.update(0x78, true));
        assert_eq!(m, ModifierState::default());
    }

    #[test]
    fn snapshot_reads_current_keys() {
        let m = ModifierState::snapshot(|vk| vk == LWIN || vk == LALT);
        assert!(m.matches(&combo("Alt+Win+F1")));
    }
}
