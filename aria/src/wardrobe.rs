//! Wardrobe: the props the user has loaded, and where they attach.
//!
//! Nothing is built in. A prop is any glTF/GLB (or VRM) on disk, attached to one of the bones in
//! [`BONES`], with an offset, rotation and scale. The list is persisted as `avatar_props` in
//! settings.json and pushed to the renderer as a whole whenever it changes.

use aria_protocol::PropSpec;

/// The bones a prop may hang from, with the label the picker shows.
pub const BONES: &[(&str, &str)] = &[
    ("head", "Head — hats, ears, halos"),
    ("upperChest", "Upper chest — back props, wings, swords"),
    ("chest", "Chest"),
    ("spine", "Spine — waist level"),
    ("hips", "Hips — tails, belts"),
    ("leftHand", "Left hand"),
    ("rightHand", "Right hand"),
    ("leftUpperArm", "Left upper arm"),
    ("rightUpperArm", "Right upper arm"),
    ("leftShoulder", "Left shoulder"),
    ("rightShoulder", "Right shoulder"),
    ("leftUpperLeg", "Left thigh"),
    ("rightUpperLeg", "Right thigh"),
];

/// The label for a bone, falling back to its wire name.
pub fn bone_label(key: &str) -> &str {
    BONES.iter().find(|(k, _)| *k == key).map(|(_, l)| *l).unwrap_or(key)
}

/// The last path component, for the list.
pub fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

/// Converts one persisted entry into a spec. Entries written by older versions were plain
/// booleans for built-in props; those are ignored, since the built-ins no longer exist.
pub fn from_settings(key: &str, value: &serde_json::Value) -> Option<PropSpec> {
    let object = value.as_object()?;
    let path = object.get("path")?.as_str()?.trim().to_string();
    if path.is_empty() {
        return None;
    }
    let vec3 = |name: &str, default: [f32; 3]| -> [f32; 3] {
        object
            .get(name)
            .and_then(|v| v.as_array())
            .and_then(|a| {
                let n: Vec<f32> = a.iter().filter_map(|v| v.as_f64().map(|f| f as f32)).collect();
                (n.len() == 3).then(|| [n[0], n[1], n[2]])
            })
            .unwrap_or(default)
    };
    Some(PropSpec {
        id: key.to_string(),
        path,
        bone: object.get("bone").and_then(|v| v.as_str()).unwrap_or("head").to_string(),
        offset: vec3("offset", [0.0; 3]),
        rotation: vec3("rotation", [0.0; 3]),
        scale: object.get("scale").and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(1.0),
        enabled: object.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
    })
}

/// Every prop currently configured, in a stable order.
pub fn all(settings: &crate::config::Settings) -> Vec<PropSpec> {
    settings
        .avatar_props
        .iter()
        .filter_map(|(key, value)| from_settings(key, value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises one spec for persistence.
    fn to_settings(spec: &PropSpec) -> serde_json::Value {
        serde_json::json!({
            "path": spec.path,
            "bone": spec.bone,
            "offset": spec.offset,
            "rotation": spec.rotation,
            "scale": spec.scale,
            "enabled": spec.enabled,
        })
    }

    #[test]
    fn a_saved_prop_comes_back_unchanged() {
        let spec = PropSpec {
            id: "hat".into(),
            path: "C:/props/hat.glb".into(),
            bone: "head".into(),
            offset: [0.0, 0.12, -0.03],
            rotation: [0.0, 20.0, 0.0],
            scale: 1.4,
            enabled: false,
        };
        let back = from_settings("hat", &to_settings(&spec)).expect("round trips");
        assert_eq!(back, spec);
    }

    #[test]
    fn old_boolean_entries_and_junk_are_ignored() {
        // The built-in wardrobe predates user props and stored `{ "halo": true }`.
        assert!(from_settings("halo", &serde_json::json!(true)).is_none());
        assert!(from_settings("halo", &serde_json::json!(false)).is_none());
        assert!(from_settings("x", &serde_json::json!({"path": "   "})).is_none());
        assert!(from_settings("x", &serde_json::json!({})).is_none());
    }

    #[test]
    fn missing_transform_fields_get_defaults() {
        let spec = from_settings("cup", &serde_json::json!({"path": "c:/cup.glb"})).expect("parsed");
        assert_eq!(spec.bone, "head");
        assert_eq!(spec.offset, [0.0; 3]);
        assert_eq!(spec.scale, 1.0, "a prop with no scale is life size");
        assert!(spec.enabled);
    }

    #[test]
    fn every_offered_bone_has_a_label() {
        for (key, label) in BONES {
            assert!(!key.is_empty() && !label.is_empty());
            assert_eq!(bone_label(key), *label);
        }
        assert_eq!(bone_label("made_up"), "made_up");
    }
}
