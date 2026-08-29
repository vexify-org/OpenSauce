//! Keybinding layer — mirrors opencode's leader-key (`ctrl+x`) TUI bindings.
//!
//! An [`Action`] is bound to one or more [`Chord`]s (alternatives). A chord may
//! be prefixed by the *leader* key (`ctrl+x`, configurable). Holding leader then
//! pressing the follow-up key fires the action — exactly how opencode works.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The set of TUI actions OpenSauce supports. Mirrors opencode's `tui.json`
/// bis the ones relevant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    AppQuit,
    CommandList,
    SessionNew,
    SessionList,
    SessionExport,
    SessionInterrupt, // abort the in-flight generation (escape)
    SessionCompact,
    ModelList,
    AgentCycle,    // tab: switch Build/Plan
    EditorOpen,    // compose the prompt in $EDITOR
    ToggleDetails, // show/hide tool detail bodies
    ToggleSidebar,
    HelpShow,
    Share,
    Themes,
    Init,
    Thinking,
    AutoApprove,
    InputSubmit,
    InputNewline,
    InputClear,
    CopyToClipboard,
    Undo,
    Redo,
}

/// A single key press, normalized. `via_leader` means it follows the leader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub key: KeyCode,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub via_leader: bool,
}

impl Chord {
    fn new(key: KeyCode) -> Self {
        Chord { key, ctrl: false, shift: false, alt: false, via_leader: false }
    }
    fn ctrl(key: KeyCode) -> Self {
        Chord { key, ctrl: true, shift: false, alt: false, via_leader: false }
    }
}

pub struct Binding {
    pub action: Action,
    pub chords: Vec<Chord>,
}

/// Leader key — the default is `ctrl+x`, matching opencode.
pub fn leader_chord() -> Chord {
    Chord::ctrl(KeyCode::Char('x'))
}

/// Default bindings (opencode-style). Later entries win on conflict.
pub fn default_bindings() -> Vec<Binding> {
    use Action::*;
    let mut b = Vec::new();
    let mut add = |action: Action, chords: Vec<Chord>, leader: Vec<Chord>| {
        let mut all = chords;
        for c in leader {
            let mut c = c;
            c.via_leader = true;
            all.push(c);
        }
        b.push(Binding { action, chords: all });
    };

    // App lifecycle
    add(AppQuit, vec![Chord::ctrl(KeyCode::Char('c')), Chord::ctrl(KeyCode::Char('d'))], vec![Chord::new(KeyCode::Char('q'))]);
    // Command palette
    add(CommandList, vec![Chord::ctrl(KeyCode::Char('p'))], vec![]);
    // Sessions
    add(SessionNew, vec![], vec![Chord::new(KeyCode::Char('n'))]);
    add(SessionList, vec![], vec![Chord::new(KeyCode::Char('l'))]);
    add(SessionExport, vec![], vec![Chord::new(KeyCode::Char('x'))]);
    add(SessionCompact, vec![], vec![Chord::new(KeyCode::Char('c'))]);
    // Interrupt the running turn
    add(SessionInterrupt, vec![Chord::new(KeyCode::Esc)], vec![]);
    // Models
    add(ModelList, vec![], vec![Chord::new(KeyCode::Char('m'))]);
    // Agents / modes — opencode: ctrl+x a, or Tab to cycle
    add(AgentCycle, vec![Chord::new(KeyCode::Tab), Chord::ctrl(KeyCode::Char('m'))], vec![Chord::new(KeyCode::Char('a'))]);
    // App-ish commands
    add(Share, vec![], vec![Chord::new(KeyCode::Char('s'))]);
    add(Themes, vec![], vec![Chord::new(KeyCode::Char('t'))]);
    add(Init, vec![], vec![Chord::new(KeyCode::Char('i'))]);
    add(HelpShow, vec![], vec![Chord::new(KeyCode::Char('h'))]);
    // Prompt editing
    add(EditorOpen, vec![], vec![Chord::new(KeyCode::Char('e'))]);
    add(InputSubmit, vec![Chord::new(KeyCode::Enter)], vec![]);
    add(InputNewline, vec![Chord::ctrl(KeyCode::Char('j'))], vec![]);
    add(InputClear, vec![Chord::ctrl(KeyCode::Char('u'))], vec![]);
    // Misc
    add(Thinking, vec![], vec![Chord::new(KeyCode::Char('y'))]);
    add(ToggleDetails, vec![], vec![Chord::new(KeyCode::Char('d'))]);
    add(ToggleSidebar, vec![], vec![Chord::new(KeyCode::Char('b'))]);
    add(Undo, vec![], vec![Chord::new(KeyCode::Char('u'))]);
    add(Redo, vec![], vec![Chord::new(KeyCode::Char('r'))]);
    // Copy isn't leader-bound by default.
    b
}

/// Normalize a raw crossterm key into a `Chord`, or `None` for chord-state keys.
pub fn chord_of(key: KeyEvent) -> Option<Chord> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let code = match key.code {
        // Enter/Return aliases
        KeyCode::Enter | KeyCode::Char('\n') => KeyCode::Enter,
        // Map `Shift+Tab` to BackTab so it round-trips as Tab+shift
        KeyCode::BackTab => return Some(Chord { key: KeyCode::Tab, ctrl, shift: true, alt, via_leader: false }),
        other => other,
    };
    Some(Chord { key: code, ctrl, shift, alt, via_leader: false })
}

/// Resolve a pressed chord (with `leader_pending` from a previous leader press)
/// into the action it triggers.
pub fn resolve(action_chords: &[Binding], chord: Chord, leader_pending: bool) -> Option<Action> {
    for b in action_chords {
        for c in &b.chords {
            // A leadered bind fires only when the leader is pending.
            if c.via_leader != leader_pending {
                continue;
            }
            if c.key == chord.key
                && c.ctrl == chord.ctrl
                && c.shift == chord.shift
                && c.alt == chord.alt
            {
                return Some(b.action);
            }
        }
    }
    None
}

/// True if `chord` is the leader key itself.
pub fn is_leader(chord: &Chord) -> bool {
    let l = leader_chord();
    l.key == chord.key && l.ctrl == chord.ctrl
}

#[cfg(test)]
mod tests {
    use super::*;

    fn char_c(ctrl: bool) -> Chord {
        Chord { key: KeyCode::Char('c'), ctrl, shift: false, alt: false, via_leader: false }
    }

    #[test]
    fn insufficient_control_does_not_quit() {
        // Ctrl+C quits the app.
        assert_eq!(resolve(&default_bindings(), char_c(true), false), Some(Action::AppQuit));
        // Plain 'c' (no ctrl) must not quit — resolves to nothing here.
        assert_ne!(resolve(&default_bindings(), char_c(false), false), Some(Action::AppQuit));
    }

    #[test]
    fn leader_binds_only_fire_after_leader() {
        let binds = default_bindings();
        let n = Chord { key: KeyCode::Char('n'), ctrl: false, shift: false, alt: false, via_leader: false };
        // leader:pending needed for new-session (bound as <leader>n)
        assert_eq!(resolve(&binds, n, true), Some(Action::SessionNew));
        assert_ne!(resolve(&binds, n, false), Some(Action::SessionNew));
    }
}