//! Keyboard shortcuts. The table lives in assets/keybinds/keybinds.json and
//! follows DaVinci Resolve's defaults (docs/KEYBINDS.md).

use gtk4 as gtk;
use gtk4::prelude::*;
use serde::Deserialize;

const JSON: &str = include_str!("../../../assets/keybinds/keybinds.json");

#[derive(Debug, Clone, Deserialize)]
pub struct Keybind {
    pub group: String,
    pub action: String,
    pub keys: Vec<String>,
    #[serde(default)]
    pub alt: Vec<String>,
    pub label: String,
    pub resolve: Option<String>,
}

#[derive(Deserialize)]
struct File {
    keybinds: Vec<Keybind>,
}

pub fn load() -> Vec<Keybind> {
    match serde_json::from_str::<File>(JSON) {
        Ok(f) => f.keybinds,
        Err(e) => {
            tracing::error!("keybinds.json is invalid: {e}");
            Vec::new()
        }
    }
}

/// Human-readable form of the first key of an action, for tooltips and lists.
pub fn display(accel: &str) -> String {
    match gtk::accelerator_parse(accel) {
        Some((key, mods)) => gtk::accelerator_get_label(key, mods).to_string(),
        None => accel.to_string(),
    }
}

/// Keys that lists, drop-downs and switches use themselves.
const NAVIGATION: [&str; 10] = ["Up", "Down", "Left", "Right", "space", "Return", "Home", "End", "Page_Up", "Page_Down"];

/// Whether a key press belongs to the widget that has the focus and not to
/// the editor: the user is typing, or a dialog or menu is open, or the focus is
/// in a list and the key moves around in it.
fn belongs_to_focus(focus: &gtk::Widget, accel: &str) -> bool {
    if focus.is::<gtk::Text>() || focus.is::<gtk::TextView>() || focus.is::<gtk::Editable>() {
        return true;
    }
    // Anything open on top of the editor keeps all of its keys.
    if focus.ancestor(gtk::Popover::static_type()).is_some() || focus.ancestor(libadwaita::Dialog::static_type()).is_some() {
        return true;
    }
    let in_list = [gtk::GridView::static_type(), gtk::ListView::static_type(), gtk::DropDown::static_type(), gtk::ListBox::static_type()]
        .into_iter()
        .any(|t| focus.type_().is_a(t) || focus.ancestor(t).is_some());
    in_list && NAVIGATION.contains(&accel)
}

/// Register every binding on the window. `run` receives the action name and
/// says whether it did anything with it; a key nobody used is passed on.
pub fn install(window: &impl IsA<gtk::Window>, run: impl Fn(&str) -> bool + Clone + 'static) {
    let controller = gtk::ShortcutController::new();
    controller.set_scope(gtk::ShortcutScope::Global);
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    for bind in load() {
        for accel in bind.keys.iter().chain(bind.alt.iter()) {
            let Some(trigger) = gtk::ShortcutTrigger::parse_string(accel) else {
                tracing::warn!("bad accelerator {accel} for {}", bind.action);
                continue;
            };
            let run = run.clone();
            let action_name = bind.action.clone();
            let window_weak = window.as_ref().downgrade();
            let accel = accel.clone();
            let action = gtk::CallbackAction::new(move |_, _| {
                let elsewhere = window_weak
                    .upgrade()
                    .and_then(|w| gtk::prelude::GtkWindowExt::focus(&w))
                    .is_some_and(|focus| belongs_to_focus(&focus, &accel));
                if elsewhere || !run(&action_name) {
                    return glib::Propagation::Proceed;
                }
                glib::Propagation::Stop
            });
            controller.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
        }
    }
    window.as_ref().add_controller(controller);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Resolve keys Tempo must never use for something else (docs/KEYBINDS.md §14).
    const RESERVED: &[&str] = &["r", "w", "s", "e", "u", "f", "g", "<Primary>d", "<Primary>n", "<Primary>r", "<Primary>y", "<Primary>e", "<Primary><Shift>c"];

    #[test]
    fn no_key_is_used_twice_and_reserved_keys_stay_free() {
        let mut seen: HashMap<String, String> = HashMap::new();
        for bind in load() {
            for key in bind.keys.iter().chain(bind.alt.iter()) {
                let norm = key.to_lowercase();
                assert!(!RESERVED.contains(&norm.as_str()), "{key} is reserved but bound to {}", bind.action);
                if let Some(other) = seen.insert(norm, bind.action.clone()) {
                    assert_eq!(other, bind.action, "{key} is bound to both {other} and {}", bind.action);
                }
            }
        }
        assert!(!seen.is_empty());
    }

    /// Turn a GTK accelerator into the spelling used by Resolve's key export.
    fn resolve_spelling(accel: &str) -> String {
        let mut mods = Vec::new();
        let mut rest = accel;
        while let Some(end) = rest.strip_prefix('<').and_then(|r| r.find('>')) {
            mods.push(match &rest[1..=end] {
                "Primary" => "Ctrl",
                "Shift" => "Shift",
                "Alt" => "Alt",
                other => panic!("unknown modifier {other}"),
            });
            rest = &rest[end + 2..];
        }
        let key = match rest {
            "space" => "Space".to_string(),
            "BackSpace" => "Backspace".to_string(),
            "Delete" => "Del".to_string(),
            "bracketleft" => "[".to_string(),
            "bracketright" => "]".to_string(),
            "comma" => ",".to_string(),
            "period" => ".".to_string(),
            "backslash" => "\\".to_string(),
            "equal" => "=".to_string(),
            "minus" => "-".to_string(),
            k if k.len() == 1 => k.to_uppercase(),
            k => k.to_string(),
        };
        mods.sort();
        mods.push(&key);
        mods.join("+")
    }

    fn normalise(resolve_key: &str) -> String {
        let mut parts: Vec<&str> = resolve_key.trim().split('+').filter(|p| !p.is_empty()).collect();
        // A trailing "+" means the key itself is "+".
        if resolve_key.trim().ends_with('+') {
            parts.push("+");
        }
        let key = parts.pop().unwrap_or_default();
        let key = if key.len() == 1 { key.to_uppercase() } else { key.to_string() };
        parts.sort();
        parts.push(&key);
        parts.join("+")
    }

    #[test]
    fn keys_match_the_davinci_resolve_export() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../EX/DaVinci Resolve Keys.txt");
        let Ok(text) = std::fs::read_to_string(&path) else { return };
        let resolve: HashMap<&str, Vec<String>> = text
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once(":="))
            .map(|(name, keys)| (name.trim(), keys.split('|').map(normalise).filter(|k| !k.is_empty()).collect()))
            .collect();

        for bind in load() {
            let Some(name) = &bind.resolve else { continue };
            let expected = resolve.get(name.as_str()).unwrap_or_else(|| panic!("{name} is not in the Resolve export"));
            for key in &bind.keys {
                let ours = resolve_spelling(key);
                assert!(expected.contains(&ours), "{}: Tempo has {ours}, Resolve has {expected:?}", bind.action);
            }
        }
    }
}
