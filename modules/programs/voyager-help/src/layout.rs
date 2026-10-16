use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

const POSITIONS: usize = 52;
// QMK's layer_state_t supports at most 32 layers. The route search is bounded
// independently of that limit: unusual layer graphs must not freeze the picker.
const MAX_LAYERS: usize = 32;
const MAX_STATES: usize = 8192;
const GESTURES: [&str; 4] = ["tap", "hold", "doubleTap", "tapHold"];

#[derive(Debug)]
pub struct Layout {
    pub layout: String,
    pub revision: String,
    pub title: String,
    pub profile: Option<crate::profile::SwapProfile>,
    layers: Vec<Layer>,
}

#[derive(Debug)]
pub struct Entry {
    pub name: String,
    pub gesture: String,
    pub layer: String,
    pub aliases: Vec<String>,
}

#[derive(Debug)]
struct Layer {
    title: String,
    keys: Vec<Key>,
}

#[derive(Debug)]
struct Key {
    actions: [Option<Action>; 4],
    custom_label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Action {
    code: String,
    #[serde(default)]
    layer: Option<usize>,
    #[serde(default)]
    modifier: Option<Value>,
    #[serde(default)]
    modifiers: Option<BTreeMap<String, bool>>,
    #[serde(default, rename = "macro")]
    macro_: Option<Value>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

impl Key {
    fn transparent(&self) -> bool {
        self.actions.iter().all(|a| {
            a.as_ref().is_none_or(|a| {
                matches!(a.code.as_str(), "KC_TRANSPARENT" | "KC_TRNS" | "_______")
                    && a.layer.is_none()
                    && a.modifier.is_none()
                    && a.modifiers
                        .as_ref()
                        .is_none_or(|m| m.iter().all(|(k, v)| !v && modifier_name(k).is_some()))
                    && a.macro_.is_none()
                    && a.color.is_none()
                    && a.extra.values().all(Value::is_null)
            })
        })
    }

    fn dance(&self) -> bool {
        self.actions[2].is_some()
            || self.actions[3].is_some()
            || self.actions[1]
                .as_ref()
                .is_some_and(|a| a.code != "MO" && modifier_name(&a.code).is_none())
    }

    fn sustained_tap(&self) -> bool {
        self.actions[1].is_none() && !self.dance()
    }
}

pub fn parse_response(
    bytes: &[u8],
    expected_layout: &str,
    expected_revision: &str,
) -> Result<Layout, String> {
    let root: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid Oryx JSON: {e}"))?;
    if let Some(errors) = root.get("errors")
        && !errors.is_null()
        && errors.as_array().is_none_or(|e| !e.is_empty())
    {
        return Err(format!("Oryx GraphQL errors: {errors}"));
    }
    let data = root
        .pointer("/data/layout")
        .ok_or("Oryx response has no layout")?;
    let layout = required_string(data, "hashId")?;
    let revision_data = data
        .get("revision")
        .ok_or("Oryx response has no revision")?;
    let revision = required_string(revision_data, "hashId")?;
    if layout != expected_layout || revision != expected_revision {
        return Err(format!(
            "Oryx identity mismatch: requested {expected_layout}/{expected_revision}, received {layout}/{revision}"
        ));
    }
    // The production query is geometry=voyager. Also reject a contradictory
    // geometry if a fixture or a future API response includes this field.
    for object in [data, revision_data] {
        if let Some(geometry) = object.get("geometry")
            && geometry.as_str() != Some("voyager")
        {
            return Err("Oryx response is not Voyager geometry".into());
        }
    }
    let title = required_string(revision_data, "title")?.to_owned();
    let raw_layers = revision_data
        .get("layers")
        .and_then(Value::as_array)
        .ok_or("Oryx layers must be an array")?;
    if raw_layers.is_empty() || raw_layers.len() > MAX_LAYERS {
        return Err("Oryx must contain 1–32 Voyager layers".into());
    }
    let mut layers = Vec::with_capacity(raw_layers.len());
    for (index, raw) in raw_layers.iter().enumerate() {
        let title = required_string(raw, "title")?.to_owned();
        let keys = raw
            .get("keys")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("Layer {index}: keys must be a JSON array"))?;
        if keys.len() != POSITIONS {
            return Err(format!(
                "Layer {index}: expected 52 Voyager positions, found {}",
                keys.len()
            ));
        }
        let mut parsed = Vec::with_capacity(POSITIONS);
        for (position, key) in keys.iter().enumerate() {
            let object = key
                .as_object()
                .ok_or_else(|| format!("Layer {index}, key {position}: expected key object"))?;
            for (field, value) in object {
                if !value.is_null()
                    && !matches!(
                        field.as_str(),
                        "tap"
                            | "hold"
                            | "doubleTap"
                            | "tapHold"
                            | "icon"
                            | "about"
                            | "emoji"
                            | "glowColor"
                            | "lockGlowColor"
                            | "customLabel"
                            | "tappingTerm"
                            | "aboutPosition"
                    )
                {
                    return Err(format!(
                        "Layer {index}, key {position}: unsupported key field {field}"
                    ));
                }
            }
            let custom_label = object
                .get("customLabel")
                .filter(|value| !value.is_null())
                .map(|value| {
                    value.as_str().ok_or_else(|| {
                        format!("Layer {index}, key {position}: custom label must be text")
                    })
                })
                .transpose()?
                .filter(|label| !label.is_empty())
                .map(str::to_owned);
            let mut actions = [None, None, None, None];
            for (gesture, field) in GESTURES.iter().enumerate() {
                let Some(value) = object.get(*field).filter(|v| !v.is_null()) else {
                    continue;
                };
                let action = Action::deserialize(value).map_err(|e| {
                    format!("Layer {index}, key {position}, {field}: malformed action: {e}")
                })?;
                if action.code.trim().is_empty() {
                    return Err(format!("Layer {index}, key {position}: empty action code"));
                }
                if action.layer.is_some_and(|n| n >= raw_layers.len()) {
                    return Err(format!(
                        "Layer {index}, key {position}: target layer is out of range"
                    ));
                }
                if matches!(
                    action.code.as_str(),
                    "MO" | "TO" | "TG" | "DF" | "OSL" | "TT" | "LM" | "LT" | "PDF"
                ) && action.layer.is_none()
                {
                    return Err(format!(
                        "Layer {index}, key {position}: {} needs a target layer",
                        action.code
                    ));
                }
                if let Some(color) = &action.color
                    && (color.len() != 7
                        || !color.starts_with('#')
                        || !color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit))
                {
                    return Err(format!("Layer {index}, key {position}: invalid RGB color"));
                }
                actions[gesture] = Some(action);
            }
            parsed.push(Key {
                actions,
                custom_label,
            });
        }
        layers.push(Layer {
            title,
            keys: parsed,
        });
    }
    Ok(Layout {
        layout: layout.to_owned(),
        revision: revision.to_owned(),
        title,
        profile: None,
        layers,
    })
}

fn required_string<'a>(object: &'a Value, field: &str) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Oryx {field} must be a string"))
}

pub fn apply_profile(
    layout: &mut Layout,
    profile: crate::profile::SwapProfile,
) -> Result<(), String> {
    profile.validate()?;
    if profile.layout != layout.layout || profile.revision != layout.revision {
        return Err("Custom profile does not match the Oryx layout/revision".into());
    }
    if layout
        .layers
        .get(profile.swap_layer)
        .is_none_or(|layer| !layer.keys.iter().all(Key::transparent))
    {
        return Err("Custom mirror layer must exist and be entirely transparent".into());
    }
    if let (Some(nav), Some(alias)) = (profile.nav_layer, profile.alias_layer)
        && (nav >= layout.layers.len() || alias < layout.layers.len())
    {
        return Err("Custom navigation must exist and its alias must be unused".into());
    }
    layout.profile = Some(profile);
    Ok(())
}

// Coordinates are the Oryx Voyager geometry: left 24 + 2 thumbs, then right
// 24 + 2 thumbs. Columns are counted left-to-right on each half, as viewed
// while typing; thumb numbers likewise follow the public Oryx ordering.
fn position(index: usize) -> String {
    let side = if index < 26 { "left" } else { "right" };
    let local = index % 26;
    if local < 24 {
        format!("{side} row {} col {}", local / 6 + 1, local % 6 + 1)
    } else {
        format!("{side} thumb {}", local - 23)
    }
}

fn label(layout: &Layout, index: usize) -> String {
    let legend = layout.layers[0].keys[index].actions[0]
        .as_ref()
        .map(|action| {
            let name = action_name(action).0;
            let mods = modifiers(action).unwrap_or_default();
            if mods.is_empty() {
                name
            } else {
                format!("{} + {name}", mods.join(" + "))
            }
        })
        .unwrap_or_else(|| "unlabelled".into());
    format!("{legend} [{}]", position(index))
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct State {
    active: u32,
    default: u8,
    // 0 = up, 255 = a held persistent gateway, otherwise target layer + 1.
    // A momentary release clears the layer; QMK does not reference-count it.
    held: [u8; POSITIONS],
    // Physical keys remain down even if swap changes. QMK also caches the
    // layer at the mapped press position: do not reuse that slot while held.
    pressed_at: [u8; POSITIONS],
    // Canonical pedal entry only: 1 = mirror, 2 = mirror + navigation.
    pedal: u8,
}

impl State {
    fn neutral() -> Self {
        Self {
            active: 0,
            default: 0,
            held: [0; POSITIONS],
            pressed_at: [0; POSITIONS],
            pedal: 0,
        }
    }
}

#[derive(Clone, Copy)]
enum Step {
    Action {
        layer: usize,
        key: usize,
        mapped: usize,
        gesture: usize,
    },
    Release(usize),
    Pedal(u8),
    ReleasePedals,
}

struct Node {
    state: State,
    parent: Option<(usize, Step)>,
}

#[derive(Clone, Copy)]
struct ShiftRoute {
    node: usize,
    physical: usize,
    gesture: usize,
}

fn normalize(layout: &Layout, state: &mut State) {
    if let Some(profile) = &layout.profile
        && let (Some(nav), Some(alias)) = (profile.nav_layer, profile.alias_layer)
        && state.active & (1 << alias) != 0
    {
        state.active = (state.active & !(1 << alias)) | (1 << nav) | (1 << profile.swap_layer);
    }
}

fn mirrored(layout: &Layout, state: &State) -> bool {
    layout
        .profile
        .as_ref()
        .is_some_and(|profile| state.active & (1 << profile.swap_layer) != 0)
}

fn mapped_position(layout: &Layout, state: &State, physical: usize) -> usize {
    if mirrored(layout, state) {
        layout.profile.as_ref().unwrap().position_map[physical]
    } else {
        physical
    }
}

fn available(state: &State, physical: usize, mapped: usize) -> bool {
    state.held[physical] == 0
        && !state
            .held
            .iter()
            .zip(&state.pressed_at)
            .any(|(&held, &position)| held != 0 && position as usize == mapped)
}

fn release(layout: &Layout, mut state: State, physical: usize) -> State {
    if state.held[physical] != u8::MAX {
        state.active &= !(1 << (state.held[physical] - 1));
    }
    state.held[physical] = 0;
    state.pressed_at[physical] = 0;
    normalize(layout, &mut state);
    state
}

fn release_pedals(layout: &Layout, mut state: State) -> Option<State> {
    let profile = layout
        .profile
        .as_ref()
        .expect("pedal routes require a profile");
    let mut cleared = 1 << profile.swap_layer;
    if state.pedal == 2 {
        cleared |= 1
            << profile
                .nav_layer
                .expect("combined pedal route has navigation");
        // Two releases may be coalesced (unset only the pedal bits) or may
        // pass through layer_move(nav/mirror), dropping other keyboard layers.
        // Do not invent one deterministic result when those outcomes differ.
        if state.active & !cleared != 0 {
            return None;
        }
    }
    state.active &= !cleared;
    state.pedal = 0;
    Some(state)
}

fn resolved(layout: &Layout, state: &State, key: usize) -> Option<usize> {
    let key = mapped_position(layout, state, key);
    let active = state.active | (1u32 << state.default) | 1;
    (0..layout.layers.len()).rev().find(|&layer| {
        active & (1u32 << layer) != 0 && !layout.layers[layer].keys[key].transparent()
    })
}

fn modifiers(action: &Action) -> Result<Vec<&'static str>, String> {
    let mut result = Vec::new();
    if let Some(value) = &action.modifier {
        let name = value
            .as_str()
            .and_then(modifier_name)
            .ok_or_else(|| format!("unsupported legacy modifier {value}"))?;
        result.push(name);
    }
    if let Some(values) = &action.modifiers {
        for (key, &enabled) in values {
            let name = modifier_name(key).ok_or_else(|| format!("unsupported modifier {key}"))?;
            if enabled && !result.contains(&name) {
                result.push(name);
            }
        }
    }
    Ok(result)
}

fn modifier_name(code: &str) -> Option<&'static str> {
    Some(match code {
        "leftCtrl" | "KC_LEFT_CTRL" | "KC_LCTL" | "MOD_LCTL" => "Left Control",
        "rightCtrl" | "KC_RIGHT_CTRL" | "KC_RCTL" | "MOD_RCTL" => "Right Control",
        "leftShift" | "KC_LEFT_SHIFT" | "KC_LSFT" | "MOD_LSFT" => "Left Shift",
        "rightShift" | "KC_RIGHT_SHIFT" | "KC_RSFT" | "MOD_RSFT" => "Right Shift",
        "leftAlt" | "KC_LEFT_ALT" | "KC_LALT" | "MOD_LALT" => "Left Alt",
        "rightAlt" | "KC_RIGHT_ALT" | "KC_RALT" | "MOD_RALT" => "Right Alt / AltGr",
        "leftGui" | "KC_LEFT_GUI" | "KC_LGUI" | "MOD_LGUI" => "Left GUI / Super",
        "rightGui" | "KC_RIGHT_GUI" | "KC_RGUI" | "MOD_RGUI" => "Right GUI / Super",
        _ => return None,
    })
}

fn unsupported(action: &Action) -> Option<String> {
    match modifiers(action) {
        Err(reason) => return Some(reason),
        Ok(modifiers) if action.layer.is_some() && !modifiers.is_empty() => {
            return Some("combined layer/modifier semantics are not modeled".into());
        }
        _ => {}
    }
    if let Some((name, value)) = action.extra.iter().find(|(_, v)| !v.is_null()) {
        return Some(format!("unsupported action field {name}={value}"));
    }
    if let Some(sequence) = &action.macro_ {
        return Some(format!(
            "macro sequence is shown as data, not interpreted: {sequence}"
        ));
    }
    if let Some(layer) = action.layer
        && !matches!(action.code.as_str(), "MO" | "TO" | "TG" | "DF")
    {
        return Some(format!(
            "unsupported layer semantics {} (target {layer})",
            action.code
        ));
    }
    if let Some(color) = &action.color
        && action.code != "RGB"
    {
        return Some(format!("unsupported color attachment {color}"));
    }
    if action.code == "RGB" && action.color.is_none() {
        return Some("RGB action has no color".into());
    }
    if !action_name(action).1 {
        return Some(format!("unknown action {}", action.code));
    }
    None
}

fn unsupported_gesture(key: &Key, gesture: usize) -> Option<String> {
    let action = key.actions[gesture].as_ref()?;
    unsupported(action).or_else(|| {
        (action.code == "MO" && (gesture == 2 || gesture == 0 && !key.sustained_tap()))
            .then(|| "momentary layer on a non-holding gesture has no modeled lifetime".into())
    })
}

fn transition(
    layout: &Layout,
    action: &Action,
    gesture: usize,
    state: State,
    key: usize,
) -> Option<State> {
    let layer = action.layer?;
    let mut next = state;
    match action.code.as_str() {
        "MO" if gesture == 0 || gesture == 1 || gesture == 3 => {
            next.active |= 1 << layer;
            next.held[key] = layer as u8 + 1;
        }
        "TO" => next.active = 1 << layer,
        "TG" => next.active ^= 1 << layer,
        "DF" => next.default = layer as u8,
        _ => return None,
    }
    if action.code != "MO" && (gesture == 1 || gesture == 3) {
        next.held[key] = u8::MAX;
    }
    if next.held[key] != 0 {
        next.pressed_at[key] = mapped_position(layout, &state, key) as u8;
    }
    normalize(layout, &mut next);
    Some(next)
}

fn invocation(
    layout: &Layout,
    layer: usize,
    index: usize,
    physical: usize,
    gesture: usize,
    labels: &[String],
) -> String {
    let key = &layout.layers[layer].keys[index];
    let action = key.actions[gesture].as_ref().unwrap();
    let verb = match gesture {
        0 if key.sustained_tap()
            && (action.code == "MO"
                || modifier_name(&action.code).is_some()
                || matches!(
                    action.code.as_str(),
                    "KC_MS_LEFT" | "KC_MS_RIGHT" | "KC_MS_UP" | "KC_MS_DOWN"
                )) =>
        {
            "Hold"
        }
        0 if action.code == "KC_F13" && key.sustained_tap() => "Press",
        0 => "Tap",
        1 => "Hold",
        2 => "Double-tap",
        _ => "Tap, then hold",
    };
    let mut text = format!("{verb} {}", labels[physical]);
    if key.dance() {
        text.push_str(
            " without interruption; wait for the tap dance to resolve before the next key",
        );
    } else if gesture == 1 {
        text.push_str(" until the hold resolves");
    }
    text
}

fn recipe(nodes: &[Node], mut node: usize, layout: &Layout, labels: &[String]) -> String {
    let mut steps = Vec::new();
    while let Some((parent, step)) = nodes[node].parent {
        steps.push(step);
        node = parent;
    }
    steps.reverse();
    steps
        .into_iter()
        .map(|step| match step {
            Step::Action {
                layer,
                key,
                mapped,
                gesture,
            } => invocation(layout, layer, mapped, key, gesture, labels),
            Step::Release(key) => format!("Release {}", labels[key]),
            Step::Pedal(1) => "Hold the left mirror pedal".into(),
            Step::Pedal(_) => {
                "Hold the mirror + navigation pedal combination (matching pedal daemon required)"
                    .into()
            }
            Step::ReleasePedals => {
                "Release the held pedals (their mirror/navigation layers clear)".into()
            }
        })
        .collect::<Vec<_>>()
        .join(", then ")
}

fn append_step(prefix: String, step: String) -> String {
    if prefix.is_empty() {
        step
    } else {
        format!("{prefix}, then {step}")
    }
}

fn layer_name(layout: &Layout, layer: usize) -> String {
    format!("{} ({layer})", layout.layers[layer].title)
}

pub fn generate(layout: &Layout) -> Result<Vec<Entry>, String> {
    let labels: Vec<_> = (0..POSITIONS).map(|i| label(layout, i)).collect();
    // Classify once, outside the bounded graph walk. No action strings or
    // modifier vectors need allocating for every candidate route.
    let plain: Vec<[[bool; 4]; POSITIONS]> = layout
        .layers
        .iter()
        .map(|layer| {
            std::array::from_fn(|key| {
                std::array::from_fn(|gesture| {
                    layer.keys[key].actions[gesture]
                        .as_ref()
                        .is_some_and(|action| {
                            unsupported_gesture(&layer.keys[key], gesture).is_none()
                                && modifiers(action).is_ok_and(|m| m.is_empty())
                        })
                })
            })
        })
        .collect();
    let mut nodes = vec![Node {
        state: State::neutral(),
        parent: None,
    }];
    let mut seen = HashSet::from([State::neutral()]);
    // Retain a shortest route in each orientation, not just the first route
    // to a logical action (which would always prefer the unswapped base).
    let variants = if layout.profile.is_some() { 2 } else { 1 };
    let mut routes = vec![vec![[[None; 4]; POSITIONS]; layout.layers.len()]; variants];
    let mut shifted_routes: Vec<Vec<[Option<ShiftRoute>; POSITIONS]>> =
        vec![vec![[None; POSITIONS]; layout.layers.len()]; variants];
    let mut exits = vec![None; layout.layers.len()];
    let mut limited = false;
    if let Some(profile) = &layout.profile {
        for pedal in 1..=2 {
            let target = if pedal == 1 {
                profile.swap_layer
            } else if let Some(alias) = profile.alias_layer {
                alias
            } else {
                continue;
            };
            let mut state = State::neutral();
            state.active = 1 << target;
            state.pedal = pedal;
            normalize(layout, &mut state);
            enqueue(
                state,
                0,
                Step::Pedal(pedal),
                &mut nodes,
                &mut seen,
                &mut limited,
            );
        }
    }
    let mut cursor = 0;
    while cursor < nodes.len() {
        let state = nodes[cursor].state;
        let variant = usize::from(mirrored(layout, &state));
        let positions: [usize; POSITIONS] =
            std::array::from_fn(|key| mapped_position(layout, &state, key));
        if state.pedal != 0
            && let Some(next) = release_pedals(layout, state)
        {
            enqueue(
                next,
                cursor,
                Step::ReleasePedals,
                &mut nodes,
                &mut seen,
                &mut limited,
            );
        }
        let visible: [Option<usize>; POSITIONS] =
            std::array::from_fn(|key| resolved(layout, &state, key));
        // Two distinct Shift positions suffice: at most one can collide with
        // the target action key.
        let mut shifts = [None; 2];
        let mut shift_count = 0;
        for key in 0..POSITIONS {
            let mapped = positions[key];
            if !available(&state, key, mapped) {
                continue;
            }
            let Some(layer) = visible[key] else { continue };
            if let Some(gesture) = (0..2).find(|&gesture| {
                plain[layer][mapped][gesture]
                    && (gesture == 1 || layout.layers[layer].keys[mapped].sustained_tap())
                    && layout.layers[layer].keys[mapped].actions[gesture]
                        .as_ref()
                        .is_some_and(|a| is_shift(&a.code))
            }) {
                shifts[shift_count] = Some((key, gesture));
                shift_count += 1;
                if shift_count == 2 {
                    break;
                }
            }
        }
        for key in 0..POSITIONS {
            if state.held[key] != 0 {
                let next = release(layout, state, key);
                enqueue(
                    next,
                    cursor,
                    Step::Release(key),
                    &mut nodes,
                    &mut seen,
                    &mut limited,
                );
                continue;
            }
            let mapped = positions[key];
            if !available(&state, key, mapped) {
                continue;
            }
            let Some(layer) = visible[key] else { continue };
            let mapping = &layout.layers[layer].keys[mapped];
            for (gesture, action) in mapping.actions.iter().enumerate() {
                let Some(action) = action else { continue };
                routes[variant][layer][mapped][gesture].get_or_insert(cursor);
                if gesture == 0
                    && shifted(&action.code).is_some()
                    && plain[layer][mapped][gesture]
                    && let Some(&(shift_key, shift_gesture)) = shifts
                        .iter()
                        .flatten()
                        .find(|(shift_key, _)| *shift_key != key)
                {
                    shifted_routes[variant][layer][mapped].get_or_insert(ShiftRoute {
                        node: cursor,
                        physical: shift_key,
                        gesture: shift_gesture,
                    });
                }
                if let Some(next) = plain[layer][mapped][gesture]
                    .then(|| transition(layout, action, gesture, state, key))
                    .flatten()
                {
                    if action.code == "MO" && state.pedal == 0 && state.held.iter().all(|&h| h == 0)
                    {
                        let target = action.layer.unwrap();
                        if state.active & (1 << target) != 0 && target != state.default as usize {
                            exits[target].get_or_insert((cursor, layer, key, mapped, gesture));
                        }
                    }
                    enqueue(
                        next,
                        cursor,
                        Step::Action {
                            layer,
                            key,
                            mapped,
                            gesture,
                        },
                        &mut nodes,
                        &mut seen,
                        &mut limited,
                    );
                }
            }
        }
        cursor += 1;
    }
    let mut entries = Vec::new();
    for variant in 0..variants {
        for (layer, mapping) in layout.layers.iter().enumerate() {
            for (key, mapping) in mapping.keys.iter().enumerate() {
                let physical = if variant == 1 {
                    layout.profile.as_ref().unwrap().position_map[key]
                } else {
                    key
                };
                if mapping.transparent() {
                    continue;
                }
                for (gesture, action) in mapping.actions.iter().enumerate() {
                    let Some(action) = action else { continue };
                    let route = routes[variant][layer][key][gesture];
                    if route.is_none()
                        && (variant == 1
                            || variants == 2 && routes[1][layer][key][gesture].is_some())
                    {
                        continue;
                    }
                    let (mut name, _) = action_name(action);
                    if let Some(target) = action.layer {
                        name.push_str(&format!(" — {}", layer_name(layout, target)));
                    }
                    let mods = modifiers(action).unwrap_or_default();
                    if !mods.is_empty() {
                        name = format!("{} + {name}", mods.join(" + "));
                    }
                    let mut aliases = aliases(action);
                    aliases.push(GESTURES[gesture].into());
                    if let Some(label) = &mapping.custom_label {
                        aliases.push(label.clone());
                    }
                    if variant == 1 {
                        aliases.push("mirrored".into());
                    }
                    let mut text = if let Some(node) = route {
                        append_step(
                            recipe(&nodes, node, layout, &labels),
                            invocation(layout, layer, key, physical, gesture, &labels),
                        )
                    } else {
                        format!(
                            "{}: assigned to {} ({}) but no neutral-base route found{}",
                            if limited {
                                "Route unresolved"
                            } else {
                                "Unreachable through supported layer transitions"
                            },
                            labels[key],
                            GESTURES[gesture],
                            if limited {
                                " within the bounded layer search"
                            } else {
                                " (shadowed, held gateway collision, or unsupported gateway)"
                            }
                        )
                    };
                    let unknown = unsupported_gesture(mapping, gesture);
                    if let Some(reason) = unknown {
                        name = format!("Unsupported: {name}");
                        text.push_str(&format!("; semantics not inferred: {reason}"));
                        aliases.push("unsupported".into());
                    } else {
                        append_notes(&mut text, action, mapping, gesture);
                    }
                    if let Some(description) =
                        action.description.as_deref().filter(|s| !s.is_empty())
                    {
                        text.push_str(&format!("; Oryx description: {description}"));
                    }
                    if route.is_some_and(|node| nodes[node].state.held.iter().any(|&h| h != 0)) {
                        text.push_str(
                            "; keep gateway keys held through the action, then release them",
                        );
                    }
                    if route.is_some_and(|node| nodes[node].state.pedal != 0) {
                        text.push_str(
                            "; keep the pedals held through the action, then release them",
                        );
                    }
                    if variant == 1 {
                        text.push_str("; mirrored physical position");
                    }
                    entries.push(Entry {
                        name,
                        gesture: text,
                        layer: layer_name(layout, layer),
                        aliases,
                    });
                }
                if variant == 0 && mapping.dance() && mapping.actions[3].is_none() {
                    let mut text = format!(
                        "{}: tap then hold (double-hold) has no assigned action",
                        labels[key]
                    );
                    text.push_str("; with a single tap assigned, an interrupted single hold falls back to it, an interrupted double-tap emits two single taps, and three or more taps emit that many single taps");
                    entries.push(Entry {
                        name: "Tap-dance timing and unassigned double-hold".into(),
                        gesture: text,
                        layer: layer_name(layout, layer),
                        aliases: vec!["tap dance".into(), "timing".into(), "double hold".into()],
                    });
                }
                if let Some(ShiftRoute {
                    node,
                    physical: shift_key,
                    gesture: shift_gesture,
                }) = shifted_routes[variant][layer][key]
                {
                    let action = mapping.actions[0].as_ref().unwrap();
                    let shift_layer = resolved(layout, &nodes[node].state, shift_key).unwrap();
                    let shift_mapped = mapped_position(layout, &nodes[node].state, shift_key);
                    let prefix = append_step(
                        recipe(&nodes, node, layout, &labels),
                        invocation(
                            layout,
                            shift_layer,
                            shift_mapped,
                            shift_key,
                            shift_gesture,
                            &labels,
                        ),
                    );
                    entries.push(Entry {
                    name: shifted(&action.code).unwrap().into(),
                    gesture: format!("{}, then tap {} once; release the Shift key without tapping, then release any gateways{}; glyph assumes host US / altgr-intl{}", prefix, labels[physical], if nodes[node].state.pedal != 0 { " and pedals" } else { "" }, if variant == 1 { "; mirrored physical position" } else { "" }),
                    layer: format!("{} + Shift", layer_name(layout, layer)),
                    aliases: vec!["symbol".into(), "punctuation".into(), "shift".into(), action.code.clone()],
                });
                }
            }
        }
    }
    for (target, exit) in exits.into_iter().enumerate() {
        if let Some((node, layer, key, mapped, gesture)) = exit {
            entries.push(Entry {
                name: format!("Leave latched layer {}", layer_name(layout, target)),
                gesture: format!("{}, then release {}; the momentary hold's release clears layer {target} (not a toggle)", append_step(recipe(&nodes, node, layout, &labels), invocation(layout, layer, mapped, key, gesture, &labels)), labels[key]),
                layer: layer_name(layout, target),
                aliases: vec!["layer".into(), "exit".into(), "unlatch".into()],
            });
        }
    }
    Ok(entries)
}

fn enqueue(
    state: State,
    parent: usize,
    step: Step,
    nodes: &mut Vec<Node>,
    seen: &mut HashSet<State>,
    limited: &mut bool,
) {
    if seen.contains(&state) {
        return;
    }
    if nodes.len() >= MAX_STATES {
        *limited = true;
        return;
    }
    seen.insert(state);
    nodes.push(Node {
        state,
        parent: Some((parent, step)),
    });
}

fn append_notes(text: &mut String, action: &Action, key: &Key, gesture: usize) {
    match action.code.as_str() {
        "MO" => text.push_str("; release this key to clear the momentary layer"),
        "TO" => text.push_str("; layer_move replaces active layers and stays latched after release, NOT a toggle"),
        "TG" => text.push_str("; toggles the target layer and remains set after release"),
        "DF" => text.push_str("; changes the default layer until reset (not an EEPROM-persistent default)"),
        "CW_TOGG" => text.push_str("; toggles word capitalization, not Caps Lock"),
        "KC_F13" => text.push_str("; precision host with active talkd: hold to invert microphone mute, release to restore; tap under 250 ms latches until the next press"),
        "KC_F14" => text.push_str("; precision host: toggles default microphone mute while unlocked; set the baseline before starting an F13 cycle"),
        "KC_AUDIO_MUTE" | "KC_MUTE" => text.push_str("; audio-output mute, not microphone mute"),
        "KC_MS_BTN1" | "KC_MS_BTN2" | "KC_MS_BTN3" | "KC_MS_BTN4" | "KC_MS_BTN5" => text.push_str("; hold for button-down/drag, release to end"),
        "KC_MS_LEFT" | "KC_MS_RIGHT" | "KC_MS_UP" | "KC_MS_DOWN" => text.push_str("; release the action key to stop pointer movement"),
        "RGB" => text.push_str("; solid RGB preset; layer-color indication can override visible colors"),
        _ => {}
    }
    if action.code.starts_with("KC_MS_WH_") {
        text.push_str("; hold to repeat scrolling");
    }
    if is_symbol(&action.code) {
        text.push_str("; glyph assumes host US / altgr-intl");
    }
    if key.dance() {
        text.push_str("; tap-dance timing matters: interruption selects single-tap fallbacks, not the configured hold/double-tap");
    } else if gesture == 0 && key.actions[1].is_some() {
        text.push_str("; tap and release before the hold resolves");
    }
}

fn is_shift(code: &str) -> bool {
    matches!(
        code,
        "KC_LEFT_SHIFT" | "KC_RIGHT_SHIFT" | "KC_LSFT" | "KC_RSFT"
    )
}

fn shifted(code: &str) -> Option<&'static str> {
    Some(match code {
        "KC_GRAVE" | "KC_GRV" => "Tilde (~)",
        "KC_1" => "Exclamation mark (!)",
        "KC_2" => "At sign (@)",
        "KC_3" => "Hash / number sign (#)",
        "KC_4" => "Dollar ($)",
        "KC_5" => "Percent (%)",
        "KC_6" => "Caret / circumflex (^)",
        "KC_7" => "Ampersand (&)",
        "KC_8" => "Asterisk (*)",
        "KC_9" => "Left parenthesis — (",
        "KC_0" => "Right parenthesis — )",
        "KC_MINUS" | "KC_MINS" => "Underscore (_)",
        "KC_EQUAL" | "KC_EQL" => "Plus (+)",
        "KC_SCLN" => "Colon (:)",
        "KC_QUOTE" | "KC_QUOT" => "Double quote (\")",
        "KC_BSLS" => "Pipe / vertical bar (|)",
        "KC_COMMA" | "KC_COMM" => "Less than (<)",
        "KC_DOT" => "Greater than (>)",
        "KC_SLASH" | "KC_SLSH" => "Question mark (?)",
        "KC_LBRC" => "Left curly brace ({)",
        "KC_RBRC" => "Right curly brace (})",
        _ => return None,
    })
}

fn is_symbol(code: &str) -> bool {
    shifted(code).is_some()
        && !code
            .strip_prefix("KC_")
            .is_some_and(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_digit())
        || matches!(
            code,
            "KC_LPRN"
                | "KC_RPRN"
                | "KC_CIRC"
                | "KC_AMPR"
                | "KC_ASTR"
                | "KC_PERC"
                | "KC_LCBR"
                | "KC_RCBR"
                | "KC_HASH"
                | "KC_AT"
                | "KC_EXLM"
                | "KC_DLR"
                | "KC_TILD"
                | "KC_UNDS"
                | "KC_PLUS"
                | "KC_COLN"
                | "KC_DQUO"
                | "KC_PIPE"
                | "KC_LABK"
                | "KC_RABK"
                | "KC_QUES"
        )
}

fn action_name(action: &Action) -> (String, bool) {
    let code = action.code.as_str();
    if let Some(name) = modifier_name(code) {
        return (format!("{name} modifier"), true);
    }
    if let Some(letter) = code.strip_prefix("KC_") {
        if letter.len() == 1
            && (letter.as_bytes()[0].is_ascii_uppercase() || letter.as_bytes()[0].is_ascii_digit())
        {
            return (letter.into(), true);
        }
        if letter
            .strip_prefix('F')
            .and_then(|f| f.parse::<u8>().ok())
            .is_some_and(|n| (1..=24).contains(&n))
        {
            return (letter.into(), true);
        }
    }
    let name = match code {
        "KC_TRANSPARENT" | "KC_TRNS" | "_______" => "Transparent (whole-key fallthrough)",
        "KC_NO" | "XXXXXXX" => "Disabled key (blocks lower layers)",
        "MO" => "Momentary layer",
        "TO" => "Move to latched layer",
        "TG" => "Toggle layer",
        "DF" => "Set default layer",
        "CW_TOGG" => "Caps Word",
        "KC_CAPS" | "KC_CAPS_LOCK" => "Caps Lock",
        "KC_ENTER" | "KC_ENT" => "Enter / Return",
        "KC_ESCAPE" | "KC_ESC" => "Escape",
        "KC_SPACE" | "KC_SPC" => "Space",
        "KC_BSPC" | "KC_BACKSPACE" => "Backspace",
        "KC_TAB" => "Tab",
        "KC_DELETE" | "KC_DEL" => "Delete forward",
        "KC_INSERT" | "KC_INS" => "Insert",
        "KC_GRAVE" | "KC_GRV" => "Backtick / grave (`)",
        "KC_EQUAL" | "KC_EQL" => "Equals (=)",
        "KC_SCLN" => "Semicolon (;)",
        "KC_QUOTE" | "KC_QUOT" => "Apostrophe / quote (')",
        "KC_MINUS" | "KC_MINS" => "Minus / hyphen (-)",
        "KC_BSLS" => "Backslash (\\)",
        "KC_COMMA" | "KC_COMM" => "Comma (,)",
        "KC_DOT" => "Period / dot (.)",
        "KC_SLASH" | "KC_SLSH" => "Slash (/)",
        "KC_LPRN" => "Left parenthesis — (",
        "KC_RPRN" => "Right parenthesis — )",
        "KC_CIRC" => "Caret / circumflex (^)",
        "KC_AMPR" => "Ampersand (&)",
        "KC_ASTR" => "Asterisk (*)",
        "KC_LBRC" => "Left square bracket ([)",
        "KC_RBRC" => "Right square bracket (])",
        "KC_PERC" => "Percent (%)",
        "KC_LCBR" => "Left curly brace ({)",
        "KC_RCBR" => "Right curly brace (})",
        "KC_HASH" => "Hash / number sign (#)",
        "KC_AT" => "At sign (@)",
        "KC_EXLM" => "Exclamation mark (!)",
        "KC_DLR" => "Dollar ($)",
        "KC_TILD" => "Tilde (~)",
        "KC_UNDS" => "Underscore (_)",
        "KC_PLUS" => "Plus (+)",
        "KC_COLN" => "Colon (:)",
        "KC_DQUO" => "Double quote (\")",
        "KC_PIPE" => "Pipe / vertical bar (|)",
        "KC_LABK" => "Less than (<)",
        "KC_RABK" => "Greater than (>)",
        "KC_QUES" => "Question mark (?)",
        "KC_LEFT" => "Left arrow",
        "KC_DOWN" => "Down arrow",
        "KC_UP" => "Up arrow",
        "KC_RIGHT" => "Right arrow",
        "KC_HOME" => "Home",
        "KC_END" => "End",
        "KC_PGDN" | "KC_PAGE_DOWN" => "Page Down",
        "KC_PGUP" | "KC_PAGE_UP" => "Page Up",
        "KC_PSCR" | "KC_PRINT_SCREEN" => "Print Screen",
        "KC_PAUSE" | "KC_PAUS" => "Pause / Break",
        "KC_SCRL" | "KC_SCROLL_LOCK" => "Scroll Lock",
        "KC_MEDIA_PREV_TRACK" | "KC_MPRV" => "Previous media track",
        "KC_MEDIA_NEXT_TRACK" | "KC_MNXT" => "Next media track",
        "KC_MEDIA_STOP" | "KC_MSTP" => "Stop media",
        "KC_MEDIA_PLAY_PAUSE" | "KC_MPLY" => "Play / pause media",
        "KC_AUDIO_VOL_DOWN" | "KC_VOLD" => "Speaker volume down",
        "KC_AUDIO_VOL_UP" | "KC_VOLU" => "Speaker volume up",
        "KC_AUDIO_MUTE" | "KC_MUTE" => "Speaker mute",
        "KC_MS_BTN1" => "Left mouse click / button 1",
        "KC_MS_BTN2" => "Right mouse click / button 2",
        "KC_MS_BTN3" => "Middle mouse click / button 3",
        "KC_MS_BTN4" => "Mouse button 4",
        "KC_MS_BTN5" => "Mouse button 5",
        "KC_MS_LEFT" => "Move pointer left",
        "KC_MS_DOWN" => "Move pointer down",
        "KC_MS_UP" => "Move pointer up",
        "KC_MS_RIGHT" => "Move pointer right",
        "KC_MS_WH_LEFT" => "Mouse wheel left",
        "KC_MS_WH_DOWN" => "Mouse wheel down",
        "KC_MS_WH_UP" => "Mouse wheel up",
        "KC_MS_WH_RIGHT" => "Mouse wheel right",
        "KC_MS_ACCEL0" => "Mouse acceleration 0",
        "KC_MS_ACCEL1" => "Mouse acceleration 1",
        "KC_MS_ACCEL2" => "Mouse acceleration 2",
        "RGB_TOG" => "Toggle RGB lighting",
        "TOGGLE_LAYER_COLOR" => "Toggle layer colors",
        "RGB_MODE_FORWARD" | "RGB_MOD" => "Next RGB mode",
        "RGB_MODE_REVERSE" | "RGB_RMOD" => "Previous RGB mode",
        "RGB_SLD" => "Solid RGB mode",
        "RGB_VAD" => "RGB brightness down",
        "RGB_VAI" => "RGB brightness up",
        "RGB_HUI" => "RGB hue up",
        "RGB_HUD" => "RGB hue down",
        "RGB_SAI" => "RGB saturation up",
        "RGB_SAD" => "RGB saturation down",
        "RGB" => {
            return (
                format!(
                    "Solid RGB color {}",
                    action.color.as_deref().unwrap_or("(missing color)")
                ),
                true,
            );
        }
        "QK_BOOT" | "RESET" => "Reboot to bootloader",
        "QK_REBOOT" => "Reboot keyboard",
        "EE_CLR" | "EEP_RST" => "Clear keyboard EEPROM",
        _ => return (code.into(), false),
    };
    (name.into(), true)
}

fn aliases(action: &Action) -> Vec<String> {
    let mut result = vec![action.code.clone()];
    let code = action.code.as_str();
    let words: &[&str] = if let Some(modifier) = modifier_name(code) {
        if modifier.contains("Control") {
            &["modifier", "control", "ctrl"]
        } else if modifier.contains("Shift") {
            &["modifier", "shift", "uppercase"]
        } else if modifier.contains("GUI") {
            &["modifier", "super", "gui", "win", "command", "meta"]
        } else if modifier.contains("AltGr") {
            &["modifier", "alt", "altgr", "level3"]
        } else {
            &["modifier", "alt", "option"]
        }
    } else if code.starts_with("KC_MS_") {
        &["mouse", "pointer", "click", "scroll", "wheel", "drag"]
    } else if code.starts_with("RGB") || code == "TOGGLE_LAYER_COLOR" {
        &["RGB", "lighting", "backlight", "color"]
    } else if code
        .strip_prefix("KC_F")
        .is_some_and(|s| s.parse::<u8>().is_ok())
    {
        &["function", "F keys"]
    } else if matches!(code, "MO" | "TO" | "TG" | "DF") {
        &["layer", "gateway"]
    } else if is_symbol(code) {
        &["symbol", "punctuation"]
    } else {
        &[]
    };
    result.extend(words.iter().map(|s| (*s).into()));
    if code
        .strip_prefix("KC_")
        .is_some_and(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_digit())
    {
        result.extend(["number".into(), "digit".into()]);
    }
    if code == "RGB"
        && let Some(color) = action.color.as_deref()
    {
        // The validated hex value remains the authoritative color. These
        // broad channel-family aliases make ordinary preset names findable.
        let rgb = u32::from_str_radix(&color[1..], 16).unwrap();
        let (r, g, b) = ((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
        if r > g && r > b {
            result.push("red".into());
        }
        if g > r && g > b {
            result.push("green".into());
        }
        if b > r && b > g {
            result.push("blue".into());
        }
    }
    for modifier in modifiers(action).unwrap_or_default() {
        result.push(modifier.into());
        if modifier.contains("Control") {
            result.push("ctrl".into());
        }
        if modifier.contains("GUI") {
            result.extend(["super".into(), "meta".into(), "command".into()]);
        }
    }
    if code == "KC_F13" || code == "KC_F14" {
        result.extend(["microphone".into(), "mic".into(), "mute".into()]);
    }
    if code == "KC_F13" {
        result.extend(["push to talk".into(), "ptt".into(), "talkd".into()]);
    }
    if code == "CW_TOGG" {
        result.extend(["caps".into(), "capitalization".into(), "uppercase".into()]);
    }
    if let Some(description) = &action.description
        && !description.is_empty()
    {
        result.push(description.clone());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(layers: usize) -> Value {
        json!({"data":{"layout":{"hashId":"layout","revision":{"hashId":"revision","title":"Test","layers": (0..layers).map(|i| json!({"title":format!("Layer {i}"),"keys":vec![json!({"tap":null,"hold":null,"doubleTap":null,"tapHold":null});52]})).collect::<Vec<_>>()}}}})
    }

    fn put(value: &mut Value, layer: usize, key: usize, gesture: &str, action: Value) {
        value["data"]["layout"]["revision"]["layers"][layer]["keys"][key][gesture] = action;
    }

    fn parse(value: &Value) -> Result<Layout, String> {
        parse_response(&serde_json::to_vec(value).unwrap(), "layout", "revision")
    }

    fn entry<'a>(entries: &'a [Entry], name: &str, layer: usize) -> &'a Entry {
        entries
            .iter()
            .find(|e| e.name == name && e.layer == format!("Layer {layer} ({layer})"))
            .unwrap()
    }

    #[test]
    fn generated_routes_never_type_through_a_held_mapped_source_cache_slot() {
        let mut value = response(7);
        put(&mut value, 0, 6, "hold", json!({"code":"MO","layer":3}));
        put(&mut value, 3, 7, "tap", json!({"code":"TG","layer":6}));
        put(&mut value, 3, 6, "tap", json!({"code":"KC_F1"}));
        let entries = generate(&custom(&value)).unwrap();
        assert!(entry(&entries, "F1", 3).gesture.starts_with("Unreachable"));
        assert!(!entries.iter().any(
            |entry| entry.name == "F1" && entry.gesture.contains("mirrored physical position")
        ));
        // A genuinely independent gateway makes both orientations possible.
        put(&mut value, 0, 8, "hold", json!({"code":"MO","layer":3}));
        let entries = generate(&custom(&value)).unwrap();
        assert!(!entry(&entries, "F1", 3).gesture.starts_with("Unreachable"));
        let mirrored = mirrored_entry(&entries, "KC_F1");
        assert!(mirrored.gesture.contains("[right row 2 col 6]"));
        assert!(mirrored.gesture.contains("until the hold resolves"));
    }

    fn profile() -> crate::profile::SwapProfile {
        // Derive test geometry from rows, not a table of user assignments.
        let mut position_map = vec![0; POSITIONS];
        for left in 0..26 {
            let right = if left < 24 {
                26 + 6 * (left / 6) + 5 - left % 6
            } else {
                75 - left
            };
            position_map[left] = right;
            position_map[right] = left;
        }
        crate::profile::SwapProfile {
            schema: 1,
            layout: "layout".into(),
            revision: "revision".into(),
            swap_layer: 6,
            nav_layer: Some(4),
            alias_layer: Some(7),
            position_map,
        }
    }

    fn custom(value: &Value) -> Layout {
        let mut layout = parse(value).unwrap();
        apply_profile(&mut layout, profile()).unwrap();
        layout
    }

    fn mirrored_entry<'a>(entries: &'a [Entry], code: &str) -> &'a Entry {
        entries
            .iter()
            .find(|entry| {
                entry.aliases.iter().any(|alias| alias == code)
                    && entry.gesture.contains("mirrored physical position")
            })
            .unwrap()
    }

    #[test]
    fn custom_routes_keep_normal_and_mirrored_actions_on_physical_legends() {
        let mut value = response(7);
        for (key, code) in [
            (0, "KC_GRAVE"),
            (31, "KC_F13"),
            (7, "KC_Q"),
            (36, "KC_SCLN"),
            (24, "KC_SPACE"),
            (51, "KC_BSPC"),
            (1, "KC_1"),
            (30, "KC_9"),
        ] {
            put(&mut value, 0, key, "tap", json!({"code":code}));
        }
        put(&mut value, 0, 51, "hold", json!({"code":"KC_LEFT_SHIFT"}));
        let entries = generate(&custom(&value)).unwrap();
        let f13 = mirrored_entry(&entries, "KC_F13");
        assert!(
            f13.gesture
                .starts_with("Hold the left mirror pedal, then Press")
        );
        assert!(f13.gesture.contains("[left row 1 col 1]"));
        assert!(entries.iter().any(|entry| entry.name == "F13"
            && entry.gesture.starts_with("Press F13 [right row 1 col 6]")));
        assert!(
            mirrored_entry(&entries, "KC_SCLN")
                .gesture
                .contains("Tap Q [left row 2 col 2]")
        );
        assert!(
            mirrored_entry(&entries, "KC_Q")
                .gesture
                .contains("[right row 2 col 5]")
        );
        assert!(
            mirrored_entry(&entries, "KC_LEFT_SHIFT")
                .gesture
                .contains("Hold Space [left thumb 1] until the hold resolves")
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "Left parenthesis — ("
                    && entry.gesture.contains("Hold Space [left thumb 1]")
                    && entry.gesture.contains("tap 1 [left row 1 col 2]"))
        );
    }

    #[test]
    fn alias_pedal_navigation_preserves_left_hand_gateway_positions() {
        let mut value = response(7);
        for (key, code) in [(13, "KC_A"), (14, "KC_R"), (15, "KC_S"), (16, "KC_T")] {
            put(&mut value, 0, key, "tap", json!({"code":code}));
        }
        // Accepted EeKlbD navigation has the arrows on the right home row.
        for (key, code) in [
            (39, "KC_LEFT"),
            (40, "KC_DOWN"),
            (41, "KC_UP"),
            (42, "KC_RIGHT"),
        ] {
            put(&mut value, 4, key, "tap", json!({"code":code}));
        }
        let layout = custom(&value);
        let entries = generate(&layout).unwrap();
        for (code, legend) in [
            ("KC_LEFT", "T"),
            ("KC_DOWN", "S"),
            ("KC_UP", "R"),
            ("KC_RIGHT", "A"),
        ] {
            let route = &mirrored_entry(&entries, code).gesture;
            assert!(route.starts_with("Hold the mirror + navigation pedal combination"));
            assert!(route.contains("matching pedal daemon required"));
            assert!(route.contains(&format!("then Tap {legend} [left row 3")));
            assert!(!route.contains("gateway keys held"));
        }
        assert!(
            !entries
                .iter()
                .any(|entry| entry.gesture.contains("mouse pedal"))
        );
        let mut state = State::neutral();
        state.active = 1 << 7;
        normalize(&layout, &mut state);
        assert_eq!(state.active, (1 << 4) | (1 << 6));
        assert_eq!(resolved(&layout, &state, 16), Some(4));
    }

    #[test]
    fn releasing_mirror_uses_press_cache_and_default_layer_is_not_swap() {
        let mut value = response(7);
        put(&mut value, 0, 6, "tap", json!({"code":"KC_EQUAL"}));
        put(&mut value, 0, 6, "hold", json!({"code":"MO","layer":6}));
        put(&mut value, 0, 37, "tap", json!({"code":"KC_QUOTE"}));
        put(&mut value, 4, 37, "tap", json!({"code":"KC_F1"}));
        let layout = custom(&value);
        let action = layout.layers[0].keys[6].actions[1].as_ref().unwrap();
        let state = transition(&layout, action, 1, State::neutral(), 6).unwrap();
        assert!(mirrored(&layout, &state));
        assert_eq!(state.pressed_at[6], 6);
        // Physical gateway and counterpart both conflict: the counterpart
        // would overwrite the mapped QMK source-layer cache slot.
        assert!(!available(&state, 6, mapped_position(&layout, &state, 6)));
        assert!(!available(&state, 37, mapped_position(&layout, &state, 37)));
        assert!(available(&state, 7, mapped_position(&layout, &state, 7)));
        let released = release(&layout, state, 6);
        assert!(!mirrored(&layout, &released));
        assert!(available(&released, 6, 6));
        assert!(available(&released, 37, 37));
        let mut default = State::neutral();
        default.default = 6;
        assert!(!mirrored(&layout, &default));
        assert_eq!(mapped_position(&layout, &default, 6), 6);
        default.active = 1 << 4;
        assert_eq!(resolved(&layout, &default, 37), Some(4));
        let labels = (0..POSITIONS)
            .map(|key| label(&layout, key))
            .collect::<Vec<_>>();
        assert!(invocation(&layout, 0, 6, 6, 1, &labels).contains("until the hold resolves"));
    }

    #[test]
    fn release_across_swap_does_not_retarget_a_held_gateway() {
        let mut value = response(7);
        put(&mut value, 0, 37, "hold", json!({"code":"MO","layer":4}));
        let layout = custom(&value);
        let mut state = State::neutral();
        state.active = 1 << 6;
        let action = layout.layers[0].keys[37].actions[1].as_ref().unwrap();
        let mut state = transition(&layout, action, 1, state, 6).unwrap();
        assert_eq!(state.pressed_at[6], 37);
        state.active &= !(1 << 6);
        assert!(!available(&state, 37, 37));
        assert!(!available(&state, 6, 6));
        let state = release(&layout, state, 6);
        assert_eq!(state.active, 0);
        assert!(available(&state, 37, 37));
    }

    #[test]
    fn releasing_pedals_preserves_keyboard_held_layers() {
        let mut value = response(7);
        put(&mut value, 0, 50, "hold", json!({"code":"MO","layer":2}));
        put(&mut value, 0, 0, "tap", json!({"code":"KC_F13"}));
        put(&mut value, 2, 0, "tap", json!({"code":"KC_F1"}));
        let layout = custom(&value);
        for pedal in [1, 2] {
            let mut state = State::neutral();
            state.active = (1 << 6) | if pedal == 2 { 1 << 4 } else { 0 };
            state.pedal = pedal;
            let action = layout.layers[0].keys[50].actions[1].as_ref().unwrap();
            let held = transition(&layout, action, 1, state, 25).unwrap();
            if pedal == 1 {
                let released = release_pedals(&layout, held).unwrap();
                assert_eq!(resolved(&layout, &released, 0), Some(2));
                assert!(!mirrored(&layout, &released));
                assert_eq!(released.held[25], 3);
            } else {
                assert!(
                    release_pedals(&layout, held).is_none(),
                    "coalesced and separate combo releases differ with another active layer"
                );
            }
        }
    }

    #[test]
    fn latched_swap_frees_a_cache_collision_only_after_gateway_release() {
        let mut value = response(7);
        put(&mut value, 0, 6, "hold", json!({"code":"TO","layer":6}));
        put(&mut value, 0, 6, "tap", json!({"code":"KC_F1"}));
        let layout = custom(&value);
        let action = layout.layers[0].keys[6].actions[1].as_ref().unwrap();
        let held = transition(&layout, action, 1, State::neutral(), 6).unwrap();
        assert!(!available(&held, 37, 6));
        let freed = release(&layout, held, 6);
        assert!(mirrored(&layout, &freed));
        assert!(available(&freed, 37, 6));
        assert_eq!(resolved(&layout, &freed, 37), Some(0));
    }

    #[test]
    fn invalid_profiles_are_rejected_without_changing_stock_semantics() {
        let value = response(7);
        let mut layout = parse(&value).unwrap();
        for change in 0..6 {
            let mut bad = profile();
            match change {
                0 => bad.revision = "other".into(),
                1 => bad.position_map[0] = 52,
                2 => bad.swap_layer = 8,
                3 => bad.alias_layer = Some(5),
                4 => bad.nav_layer = Some(8),
                _ => bad.alias_layer = None,
            }
            assert!(apply_profile(&mut layout, bad).is_err());
            assert!(layout.profile.is_none());
        }
        let mut opaque = value.clone();
        put(&mut opaque, 6, 0, "tap", json!({"code":"KC_A"}));
        assert!(apply_profile(&mut parse(&opaque).unwrap(), profile()).is_err());
        let mut disabled_alias = profile();
        disabled_alias.nav_layer = None;
        disabled_alias.alias_layer = None;
        apply_profile(&mut layout, disabled_alias).unwrap();
        assert!(
            !generate(&layout)
                .unwrap()
                .iter()
                .any(|entry| entry.gesture.contains("pedal combination"))
        );
        let mut stock = value;
        put(&mut stock, 0, 0, "tap", json!({"code":"KC_GRAVE"}));
        put(&mut stock, 0, 31, "tap", json!({"code":"KC_F13"}));
        put(&mut stock, 0, 6, "hold", json!({"code":"MO","layer":6}));
        let entries = generate(&parse(&stock).unwrap()).unwrap();
        assert_eq!(
            entries.iter().filter(|entry| entry.name == "F13").count(),
            1
        );
        assert!(
            entry(&entries, "F13", 0)
                .gesture
                .starts_with("Press F13 [right row 1 col 6]")
        );
        assert!(
            !entries
                .iter()
                .any(|entry| entry.gesture.contains("pedal") || entry.gesture.contains("mirrored"))
        );
    }

    #[test]
    fn moved_gateway_and_caps_word_follow_geometry() {
        let mut value = response(2);
        put(&mut value, 0, 50, "tap", json!({"code":"KC_ENTER"}));
        put(&mut value, 0, 50, "hold", json!({"code":"MO","layer":1}));
        put(&mut value, 0, 12, "tap", json!({"code":"KC_TAB"}));
        put(&mut value, 1, 12, "tap", json!({"code":"CW_TOGG"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        let caps = entry(&entries, "Caps Word", 1);
        assert!(caps.gesture.contains("Hold Enter / Return [right thumb 1]"));
        assert!(caps.gesture.contains("Tap Tab [left row 3 col 1]"));
        put(&mut value, 0, 50, "hold", Value::Null);
        put(&mut value, 0, 7, "hold", json!({"code":"MO","layer":1}));
        put(&mut value, 1, 12, "tap", Value::Null);
        put(&mut value, 1, 9, "tap", json!({"code":"CW_TOGG"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        let caps = entry(&entries, "Caps Word", 1);
        assert!(caps.gesture.contains("left row 2 col 2"));
        assert!(caps.gesture.contains("left row 2 col 4"));
        assert!(!caps.gesture.contains("right thumb 1"));
    }

    #[test]
    fn held_gateway_cannot_be_its_own_action_key() {
        let mut value = response(2);
        put(&mut value, 0, 0, "hold", json!({"code":"MO","layer":1}));
        put(&mut value, 1, 0, "tap", json!({"code":"KC_F1"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        assert!(entry(&entries, "F1", 1).gesture.starts_with("Unreachable"));
        put(&mut value, 0, 1, "hold", json!({"code":"MO","layer":1}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        let route = &entry(&entries, "F1", 1).gesture;
        assert!(!route.starts_with("Unreachable"));
        assert!(route.starts_with("Hold unlabelled [left row 1 col 2]"));
    }

    #[test]
    fn whole_key_precedence_blocks_lower_hold_and_shadowed_taps() {
        let mut value = response(3);
        put(&mut value, 0, 0, "tap", json!({"code":"TO","layer":2}));
        put(&mut value, 2, 1, "hold", json!({"code":"MO","layer":1}));
        put(&mut value, 2, 2, "tap", json!({"code":"KC_NO"}));
        put(&mut value, 1, 2, "tap", json!({"code":"KC_F1"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        assert!(entry(&entries, "F1", 1).gesture.starts_with("Unreachable"));
        put(&mut value, 2, 2, "tap", json!({"code":"KC_TRANSPARENT"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        assert!(!entry(&entries, "F1", 1).gesture.starts_with("Unreachable"));
        let layout = parse(&value).unwrap();
        let mut state = State::neutral();
        state.active = 1 << 2;
        assert_eq!(resolved(&layout, &state, 1), Some(2));
        assert!(layout.layers[2].keys[1].actions[0].is_none());
    }

    #[test]
    fn legacy_dance_latches_and_can_clear_its_latch() {
        let mut value = response(2);
        put(&mut value, 0, 5, "tap", json!({"code":"KC_Z"}));
        put(&mut value, 0, 5, "hold", json!({"code":"MO","layer":1}));
        put(
            &mut value,
            0,
            5,
            "doubleTap",
            json!({"code":"TO","layer":1}),
        );
        put(&mut value, 1, 6, "tap", json!({"code":"KC_F2"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        let latch = entries
            .iter()
            .find(|e| e.name.starts_with("Move to latched"))
            .unwrap();
        assert!(latch.gesture.starts_with("Double-tap Z"));
        assert!(latch.gesture.contains("NOT a toggle"));
        assert!(latch.gesture.contains("without interruption"));
        assert!(entries.iter().any(|e| e.name.starts_with("Leave latched")
            && e.gesture.contains("Double-tap Z")
            && e.gesture.contains("then release Z")));
        assert!(
            entries
                .iter()
                .any(|e| e.name == "Tap-dance timing and unassigned double-hold"
                    && e.gesture.contains("three or more taps"))
        );
        put(&mut value, 0, 5, "tapHold", json!({"code":"KC_F3"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        assert!(
            entry(&entries, "F3", 0)
                .gesture
                .starts_with("Tap, then hold Z")
        );
    }

    #[test]
    fn persistent_move_frees_gateway_for_actions() {
        let mut value = response(2);
        put(&mut value, 0, 2, "tap", json!({"code":"TO","layer":1}));
        put(&mut value, 1, 2, "tap", json!({"code":"KC_F4"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        assert!(!entry(&entries, "F4", 1).gesture.starts_with("Unreachable"));
    }

    #[test]
    fn held_persistent_gateway_is_released_before_reusing_position() {
        let mut value = response(2);
        put(&mut value, 0, 2, "hold", json!({"code":"TO","layer":1}));
        put(&mut value, 1, 2, "tap", json!({"code":"KC_F4"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        let route = &entry(&entries, "F4", 1).gesture;
        assert!(route.contains("then Release unlabelled [left row 1 col 3], then Tap"));
    }

    #[test]
    fn shift_recipes_do_not_reuse_the_target_or_a_held_gateway() {
        let mut value = response(2);
        put(&mut value, 0, 0, "tap", json!({"code":"KC_1"}));
        put(&mut value, 0, 0, "hold", json!({"code":"KC_LEFT_SHIFT"}));
        put(&mut value, 0, 1, "hold", json!({"code":"MO","layer":1}));
        put(&mut value, 1, 1, "tap", json!({"code":"KC_RIGHT_SHIFT"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        assert!(!entries.iter().any(|e| e.name == "Exclamation mark (!)"));
        put(&mut value, 0, 2, "tap", json!({"code":"KC_RIGHT_SHIFT"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        let symbol = entries
            .iter()
            .find(|e| e.name == "Exclamation mark (!)")
            .unwrap();
        assert!(symbol.gesture.contains("right Shift") || symbol.gesture.contains("Right Shift"));
        assert!(symbol.gesture.contains("left row 1 col 3"));
    }

    #[test]
    fn toggle_and_default_layers_route_without_a_held_key() {
        for code in ["TG", "DF"] {
            let mut value = response(2);
            put(&mut value, 0, 2, "tap", json!({"code":code,"layer":1}));
            put(&mut value, 1, 2, "tap", json!({"code":"KC_F4"}));
            let entries = generate(&parse(&value).unwrap()).unwrap();
            let route = &entry(&entries, "F4", 1).gesture;
            assert!(!route.starts_with("Unreachable"));
            assert!(!route.contains("gateway keys held"));
        }
    }

    #[test]
    fn changed_default_retains_transparent_base_shift_fallback() {
        let mut value = response(2);
        put(&mut value, 0, 0, "tap", json!({"code":"DF","layer":1}));
        put(&mut value, 0, 1, "tap", json!({"code":"KC_LEFT_SHIFT"}));
        put(&mut value, 1, 2, "tap", json!({"code":"KC_1"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        let symbol = entries
            .iter()
            .find(|entry| {
                entry.name == "Exclamation mark (!)" && entry.layer == "Layer 1 (1) + Shift"
            })
            .expect("DF's transparent positions still fall back to layer-zero Shift");
        assert!(symbol.gesture.contains("left row 1 col 2"));
        assert!(symbol.gesture.contains("left row 1 col 3"));
    }

    #[test]
    fn tap_modifier_with_a_different_hold_is_not_a_sustained_shift() {
        let mut value = response(1);
        put(&mut value, 0, 0, "tap", json!({"code":"KC_1"}));
        put(&mut value, 0, 1, "tap", json!({"code":"KC_LEFT_SHIFT"}));
        put(&mut value, 0, 1, "hold", json!({"code":"KC_A"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        assert!(!entries.iter().any(|e| e.name == "Exclamation mark (!)"));
        assert!(
            entry(&entries, "Left Shift modifier", 0)
                .gesture
                .starts_with("Tap ")
        );
    }

    #[test]
    fn large_layer_graph_reports_a_bound_not_false_unreachability() {
        let mut value = response(16);
        for key in 0..14 {
            put(
                &mut value,
                0,
                key,
                "hold",
                json!({"code":"MO","layer":key + 1}),
            );
        }
        put(&mut value, 15, 51, "tap", json!({"code":"KC_F1"}));
        let entries = generate(&parse(&value).unwrap()).unwrap();
        let route = &entry(&entries, "F1", 15).gesture;
        assert!(route.starts_with("Route unresolved"));
        assert!(route.contains("bounded layer search"));
    }

    #[test]
    fn metadata_modifiers_rgb_and_unknown_actions_are_not_lost() {
        let mut value = response(1);
        put(
            &mut value,
            0,
            0,
            "tap",
            json!({"code":"KC_TAB","modifiers":{"leftCtrl":true,"leftShift":true}}),
        );
        put(
            &mut value,
            0,
            1,
            "tap",
            json!({"code":"RGB","color":"#ff0000"}),
        );
        put(
            &mut value,
            0,
            2,
            "tap",
            json!({"code":"SOMETHING_NEW","description":"user label"}),
        );
        put(
            &mut value,
            0,
            3,
            "tap",
            json!({"code":"MACRO","macro":[{"code":"KC_A"}]}),
        );
        let entries = generate(&parse(&value).unwrap()).unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.name == "Left Control + Left Shift + Tab")
        );
        assert!(entries.iter().any(|e| e.name.contains("#ff0000")));
        assert!(entries.iter().any(|e| e.name == "Unsupported: SOMETHING_NEW" && e.gesture.contains("user label")));
        assert!(
            entries
                .iter()
                .any(|e| e.gesture.contains("macro sequence") && e.gesture.contains("KC_A"))
        );
        assert!(!entries.iter().any(|e| e.name == "F13" || e.name == "F14"));
    }

    #[test]
    fn malformed_and_wrong_identity_responses_fail_closed() {
        let mut value = response(1);
        assert!(parse_response(&serde_json::to_vec(&value).unwrap(), "other", "revision").is_err());
        assert!(parse_response(&serde_json::to_vec(&value).unwrap(), "layout", "latest").is_err());
        value["errors"] = json!([{"message":"partial result"}]);
        assert!(parse(&value).is_err());
        value.as_object_mut().unwrap().remove("errors");
        put(&mut value, 0, 0, "tap", json!({"code":42}));
        assert!(parse(&value).is_err());
        put(&mut value, 0, 0, "tap", json!({"code":"MO","layer":1}));
        assert!(parse(&value).is_err());
        put(
            &mut value,
            0,
            0,
            "tap",
            json!({"code":"KC_A","modifiers":{"leftCtrl":"yes"}}),
        );
        assert!(parse(&value).is_err());
        put(&mut value, 0, 0, "tap", json!({"code":"RGB","color":"red"}));
        assert!(parse(&value).is_err());
        put(&mut value, 0, 0, "tap", Value::Null);
        put(&mut value, 0, 0, "tripleTap", json!({"code":"KC_A"}));
        assert!(parse(&value).is_err());
        put(&mut value, 0, 0, "tripleTap", Value::Null);
        put(&mut value, 0, 0, "tap", Value::Null);
        value["data"]["layout"]["geometry"] = json!("moonlander");
        assert!(parse(&value).is_err());
        value["data"]["layout"]
            .as_object_mut()
            .unwrap()
            .remove("geometry");
        value["data"]["layout"]["revision"]["layers"][0]["keys"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(parse(&value).is_err());
    }
}
