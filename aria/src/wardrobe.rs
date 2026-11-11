//! Wardrobe: the accessory catalogue shared by the studio and the stage.

pub static PROPS: &[(&str, &str)] = &[
    ("angel_wings", "Seraphic wings"),
    ("witch_hat", "Witch hat"),
    ("glasses", "Anime glasses"),
    ("sunglasses", "Cyber shades"),
    ("devil_horns", "Devil horns"),
    ("cyber_sword", "Neon katana"),
    ("cat_ears", "Cyber cat ears"),
    ("headset", "RGB headset"),
    ("halo", "Cyber halo"),
    ("fox_tail", "Kitsune tail"),
    ("coffee_mug", "Coffee mug"),
    ("game_controller", "Cyber gamepad"),
    ("smartphone", "Smartphone"),
];

pub fn label(key: &str) -> &str {
    PROPS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, l)| *l)
        .unwrap_or(key)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_prop_has_a_label() {
        for (key, label) in super::PROPS {
            assert!(!key.is_empty() && !label.is_empty());
            assert_eq!(super::label(key), *label);
        }
    }
}
