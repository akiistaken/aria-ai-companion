//! Control Center wiring: pushes live state into the studio and handles every control.
//!
//! Callbacks run on the UI thread and reach the app through `post`, so no closure captures a
//! Slint component or the app handle.

use crate::app::{
    accent_tints, apply_scale, headpat, hex_to_color, poke, post, reset_pose, say, submit_prompt,
    with_app,
};
use crate::app::{AppHandle, ControlCenter};
use crate::config;
use crate::memory;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

fn choices(items: &[(&str, &str)]) -> ModelRc<crate::app::Choice> {
    let rows: Vec<crate::app::Choice> = items
        .iter()
        .map(|(key, label)| crate::app::Choice {
            key: SharedString::from(*key),
            label: SharedString::from(*label),
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

fn devices(names: &[String]) -> ModelRc<crate::app::DeviceEntry> {
    let mut rows: Vec<crate::app::DeviceEntry> = names
        .iter()
        .map(|n| crate::app::DeviceEntry {
            name: SharedString::from(n.as_str()),
        })
        .collect();
    if rows.is_empty() {
        rows.push(crate::app::DeviceEntry {
            name: SharedString::from("System default"),
        });
    }
    ModelRc::new(VecModel::from(rows))
}

fn index_of(items: &[(&str, &str)], key: &str) -> i32 {
    items
        .iter()
        .position(|(k, _)| *k == key)
        .map(|i| i as i32)
        .unwrap_or(0)
}

fn voice_choices() -> Vec<(&'static str, &'static str)> {
    crate::voice::VOICES
        .iter()
        .map(|v| (v.key, v.label))
        .collect()
}

fn lighten(color: slint::Color, amount: f32) -> slint::Color {
    let mix = |c: u8| -> u8 {
        let f = c as f32 / 255.0;
        ((f + (1.0 - f) * amount) * 255.0) as u8
    };
    slint::Color::from_rgb_u8(mix(color.red()), mix(color.green()), mix(color.blue()))
}

fn mask_key(key: &str) -> String {
    let key = key.trim();
    if key.is_empty() {
        return String::new();
    }
    if key.chars().count() > 10 {
        let start: String = key.chars().take(6).collect();
        let end: String = key
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{start}…{end}")
    } else {
        key.to_string()
    }
}

/// Pushes the current settings, mood and memory state into the studio window.
pub fn sync_studio(app: &AppHandle) {
    let studio = {
        let borrowed = app.borrow();
        let Some(studio) = borrowed.studio.as_ref() else {
            return;
        };
        studio.as_weak()
    };
    let Some(studio) = studio.upgrade() else {
        return;
    };
    let settings = config::get();
    let (mood, reason, _) = crate::mood::snapshot();
    let profile = config::persona(&mood);
    let (wash, edge) = accent_tints(profile.accent);

    studio.set_accent_color(hex_to_color(profile.accent));
    studio.set_accent_wash(hex_to_color(&wash));
    studio.set_accent_edge(hex_to_color(&edge));
    studio.set_mood_badge(profile.short_badge.into());
    studio.set_mood_name(profile.name.into());
    studio.set_mood_reason(reason.into());

    let (tier, _) = memory::affection_tier_label();
    studio.set_affection_label(tier.into());

    let emotions: Vec<crate::app::EmotionRow> = memory::all_emotion_stats()
        .into_iter()
        .map(|e| {
            let color = hex_to_color(e.color);
            crate::app::EmotionRow {
                key: SharedString::from(e.key),
                icon: SharedString::from(e.icon),
                title: SharedString::from(e.title),
                tier: SharedString::from(e.tier),
                level: SharedString::from(format!("Lv {} · {}/100", e.level, e.progress)),
                percent: (e.progress as f32 / 100.0).clamp(0.0, 1.0),
                color,
                tint: lighten(color, 0.5),
            }
        })
        .collect();
    studio.set_emotions(ModelRc::new(VecModel::from(emotions)));

    let traits = memory::traits();
    let trait_rows: Vec<crate::app::TraitRow> = memory::TRAIT_KEYS
        .iter()
        .map(|(key, name, _label, desc)| crate::app::TraitRow {
            key: SharedString::from(*key),
            name: SharedString::from(*name),
            label: SharedString::from(*name),
            desc: SharedString::from(*desc),
            percent: *traits.get(*key).unwrap_or(&70) as i32,
            color: hex_to_color(match *key {
                "openness" => "#38bdf8",
                "conscientiousness" => "#3ddc97",
                "extraversion" => "#f0b143",
                _ => "#f472b6",
            }),
        })
        .collect();
    studio.set_traits(ModelRc::new(VecModel::from(trait_rows)));

    let prop_rows: Vec<crate::app::PropCell> = crate::wardrobe::PROPS
        .iter()
        .map(|(key, label)| crate::app::PropCell {
            key: SharedString::from(*key),
            label: SharedString::from(*label),
            active: settings
                .avatar_props
                .get(*key)
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
        .collect();
    studio.set_props(ModelRc::new(VecModel::from(prop_rows)));

    let fact_rows: Vec<crate::app::FactRow> = memory::facts()
        .into_iter()
        .rev()
        .map(|f| crate::app::FactRow {
            text: SharedString::from(f.fact.as_str()),
            meta: SharedString::from(f.added_at.as_str()),
        })
        .collect();
    studio.set_facts(ModelRc::new(VecModel::from(fact_rows)));

    let voices = voice_choices();
    studio.set_voices(choices(&voices));
    studio.set_models(choices(config::AVAILABLE_MODELS));
    studio.set_framings(choices(config::BODY_FRAMINGS));
    studio.set_lightings(choices(config::STAGE_LIGHTING));
    studio.set_mic_modes(choices(config::MIC_MODES));

    studio.set_voice_index(index_of(&voices, &settings.tts_voice));
    studio.set_model_index(index_of(config::AVAILABLE_MODELS, &settings.ai_model));
    studio.set_framing_index(index_of(config::BODY_FRAMINGS, &settings.body_framing));
    studio.set_lighting_index(index_of(config::STAGE_LIGHTING, &settings.stage_lighting));
    studio.set_mic_index(index_of(config::MIC_MODES, &settings.mic_mode));

    studio.set_tts_on(settings.tts_enabled);
    studio.set_wake_word(settings.wake_word_enabled);
    studio.set_spectrum(settings.music_spectrum_enabled);
    studio.set_music_vibe(settings.music_vibe_enabled);
    studio.set_proactive(settings.proactive_enabled);
    studio.set_presence(settings.presence_guardian_enabled);
    studio.set_gaming_copilot(settings.automatic_game_copilot_enabled);
    studio.set_game_dodge(settings.smart_gaming_dodge_enabled);
    studio.set_edge_snap(settings.edge_snapping_enabled);
    studio.set_discord(settings.discord_presence_enabled);
    studio.set_subtitles(settings.subtitles_enabled);
    studio.set_chatbox(settings.input_bar_visible);
    studio.set_capsule(settings.capsule_mode);
    studio.set_capsule_autohide(settings.capsule_autohide);
    studio.set_voice_follows_mood(settings.voice_follows_mood);
    studio.set_avatar_scale(settings.avatar_scale);
    studio.set_base_url(settings.base_url.clone().into());
    studio.set_api_key_masked(mask_key(&settings.deepseek_api_key).into());
    studio.set_clipboard(app.borrow().clipboard_watch);

    studio.set_inputs(devices(&crate::audio::list_audio_devices(true)));
    studio.set_outputs(devices(&crate::audio::list_audio_devices(false)));
}

pub fn wire_studio(studio: &ControlCenter) {
    let weak = studio.as_weak();

    macro_rules! on {
        ($setter:ident, |$($arg:ident),+| $body:block) => {{
            let weak = weak.clone();
            studio.$setter(move |$($arg),+| {
                $(let $arg = $arg;)+
                if weak.upgrade().is_some() {
                    post(move |app| {
                        let _ = app;
                        $body
                    });
                }
            });
        }};
        ($setter:ident, || $body:block) => {{
            let weak = weak.clone();
            studio.$setter(move || {
                if weak.upgrade().is_some() {
                    post(move |app| {
                        let _ = app;
                        $body
                    });
                }
            });
        }};
    }

    on!(on_set_tab, |idx| {
        with_app(|app| {
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_tab(idx);
            }
        });
    });

    on!(on_toggle_tts, |on| {
        config::update(|s| s.tts_enabled = on);
        if !on {
            crate::voice::stop_speaking();
        }
        with_app(|app| {
            app.borrow().ui.set_tts(on);
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_tts_on(on);
            }
        });
    });

    on!(on_pick_voice, |idx| {
        let voices = voice_choices();
        if let Some((key, _)) = voices.get(idx.max(0) as usize) {
            config::update(|s| s.tts_voice = (*key).to_string());
            with_app(|app| {
                if let Some(s) = app.borrow().studio.as_ref() {
                    s.set_voice_index(idx);
                }
            });
        }
    });

    on!(on_audition_voice, || {
        crate::voice::speak_async(
            "Fine. Good job today. Not that I noticed! I just happened to be nearby.",
            "talking",
        );
    });

    on!(on_clone_voice_file, || {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Audio", &["mp3", "wav", "flac", "ogg"])
            .set_title("Pick a 5–20 s clip of the voice to clone")
            .pick_file()
        {
            config::update(|s| {
                s.custom_clone_audio_path = path.to_string_lossy().to_string();
            });
            with_app(|app| {
                say(
                    app,
                    &config::flav("Got your reference clip — I'll keep it for voice work."),
                    "happy",
                    vec![format!("reference clip: {}", path.display())],
                    false,
                );
            });
        }
    });

    on!(on_toggle_wake_word, |on| {
        config::update(|s| s.wake_word_enabled = on);
        if on {
            crate::voice::start_wake_word_listener(|text| {
                log::info!("wake word heard: {text}");
            });
        }
        with_app(|app| {
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_wake_word(on);
            }
        });
    });

    on!(on_pick_mic_mode, |idx| {
        if let Some((key, _)) = config::MIC_MODES.get(idx.max(0) as usize) {
            config::update(|s| s.mic_mode = (*key).to_string());
            with_app(|app| {
                if let Some(s) = app.borrow().studio.as_ref() {
                    s.set_mic_index(idx);
                }
            });
        }
    });

    on!(on_pick_input, |idx| {
        let names = crate::audio::list_audio_devices(true);
        if let Some(name) = names.get(idx.max(0) as usize) {
            config::update(|s| s.input_audio_device = name.clone());
            with_app(|app| {
                if let Some(s) = app.borrow().studio.as_ref() {
                    s.set_input_index(idx);
                }
            });
        }
    });

    on!(on_pick_output, |idx| {
        let names = crate::audio::list_audio_devices(false);
        if let Some(name) = names.get(idx.max(0) as usize) {
            config::update(|s| s.output_audio_device = name.clone());
            with_app(|app| {
                if let Some(s) = app.borrow().studio.as_ref() {
                    s.set_output_index(idx);
                }
            });
        }
    });

    macro_rules! toggle_field {
        ($callback:ident, $field:ident, $window_setter:ident) => {
            on!($callback, |on| {
                config::update(|s| s.$field = on);
                with_app(|app| {
                    if let Some(s) = app.borrow().studio.as_ref() {
                        s.$window_setter(on);
                    }
                });
            });
        };
    }

    toggle_field!(on_toggle_spectrum, music_spectrum_enabled, set_spectrum);
    toggle_field!(on_toggle_music_vibe, music_vibe_enabled, set_music_vibe);
    toggle_field!(on_toggle_proactive, proactive_enabled, set_proactive);
    toggle_field!(on_toggle_presence, presence_guardian_enabled, set_presence);
    toggle_field!(
        on_toggle_gaming,
        automatic_game_copilot_enabled,
        set_gaming_copilot
    );
    toggle_field!(on_toggle_dodge, smart_gaming_dodge_enabled, set_game_dodge);
    toggle_field!(on_toggle_edge_snap, edge_snapping_enabled, set_edge_snap);
    toggle_field!(
        on_toggle_capsule_autohide,
        capsule_autohide,
        set_capsule_autohide
    );
    toggle_field!(
        on_toggle_voice_mood,
        voice_follows_mood,
        set_voice_follows_mood
    );
    toggle_field!(on_toggle_subtitles, subtitles_enabled, set_subtitles);

    on!(on_toggle_discord, |on| {
        crate::discord::set_enabled(on);
        if on {
            crate::discord::update("", "", &config::current_persona());
        }
        with_app(|app| {
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_discord(on);
            }
        });
    });

    on!(on_toggle_clipboard, |on| {
        with_app(|app| {
            app.borrow_mut().clipboard_watch = on;
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_clipboard(on);
            }
        });
    });

    on!(on_toggle_chatbox, |on| {
        config::update(|s| s.input_bar_visible = on);
        with_app(|app| {
            app.borrow().ui.set_bar_visible(on);
            app.borrow_mut().bar_pinned_open = on;
            crate::app::sync_chrome(app);
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_chatbox(on);
            }
        });
    });

    on!(on_toggle_capsule, |on| {
        config::update(|s| s.capsule_mode = on);
        with_app(|app| {
            app.borrow().ui.set_capsule_mode(on);
            crate::app::sync_chrome(app);
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_capsule(on);
            }
        });
    });

    on!(on_pick_model, |idx| {
        if let Some((key, _)) = config::AVAILABLE_MODELS.get(idx.max(0) as usize) {
            config::update(|s| s.ai_model = (*key).to_string());
            with_app(|app| {
                if let Some(s) = app.borrow().studio.as_ref() {
                    s.set_model_index(idx);
                    s.set_status_text(format!("Model set to {key}").into());
                }
            });
        }
    });

    on!(on_save_engine, |url, key| {
        let url = url.to_string();
        let key = key.to_string();
        let is_masked = key.contains('…');
        config::update(|s| {
            if !url.trim().is_empty() {
                s.base_url = url.trim().to_string();
            }
            if !key.trim().is_empty() && !is_masked {
                s.deepseek_api_key = key.trim().to_string();
            }
        });
        if !key.trim().is_empty() && !is_masked {
            config::set_api_key(&key);
            crate::brain::brain().reset_memory();
        }
        with_app(|app| {
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_status_text("Engine settings saved.".into());
                s.set_api_key_masked(mask_key(&config::get().deepseek_api_key).into());
            }
        });
    });

    on!(on_pick_framing, |idx| {
        if let Some((key, _)) = config::BODY_FRAMINGS.get(idx.max(0) as usize) {
            config::update(|s| s.body_framing = (*key).to_string());
            with_app(|app| {
                app.borrow().stage.borrow_mut().apply_settings();
                if let Some(s) = app.borrow().studio.as_ref() {
                    s.set_framing_index(idx);
                }
            });
        }
    });

    on!(on_pick_lighting, |idx| {
        if let Some((key, _)) = config::STAGE_LIGHTING.get(idx.max(0) as usize) {
            config::update(|s| s.stage_lighting = (*key).to_string());
            with_app(|app| {
                app.borrow().stage.borrow_mut().apply_settings();
                if let Some(s) = app.borrow().studio.as_ref() {
                    s.set_lighting_index(idx);
                }
            });
        }
    });

    on!(on_set_avatar_scale, |scale| {
        with_app(|app| apply_scale(app, scale));
    });

    on!(on_reset_camera, || {
        config::update(|s| {
            s.camera_zoom = 1.0;
            s.camera_y_offset = 0.0;
        });
        with_app(|app| {
            app.borrow().stage.borrow_mut().apply_settings();
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_status_text("Camera zoom and pan reset.".into());
            }
        });
    });

    on!(on_load_vrm, || {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("VRM / glTF", &["vrm", "glb"])
            .set_title("Pick a 3D avatar model")
            .pick_file()
        {
            config::update(|s| {
                s.avatar_mode = "vrm".into();
                s.custom_vrm_path = path.to_string_lossy().to_string();
            });
            with_app(|app| switch_avatar(app, "vrm"));
        }
    });

    on!(on_load_live2d, || {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Live2D model", &["json"])
            .set_title("Pick a .model3.json")
            .pick_file()
        {
            config::update(|s| {
                s.avatar_mode = "live2d".into();
                s.live2d_source = path.to_string_lossy().to_string();
            });
            with_app(|app| switch_avatar(app, "live2d"));
        }
    });

    on!(on_load_puppet, || {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Artwork", &["png", "webp", "jpg", "jpeg"])
            .set_title("Pick character artwork")
            .pick_file()
        {
            config::update(|s| {
                s.avatar_mode = "puppet".into();
                s.puppet_source = path.to_string_lossy().to_string();
            });
            with_app(|app| switch_avatar(app, "puppet"));
        }
    });

    on!(on_toggle_prop, |key| {
        let key = key.to_string();
        let mut active = false;
        config::update(|s| {
            let current = s
                .avatar_props
                .get(&key)
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            active = !current;
            s.avatar_props
                .insert(key.clone(), serde_json::json!(active));
        });
        with_app(|app| {
            app.borrow().stage.borrow_mut().apply_settings();
            sync_studio(app);
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_status_text(
                    format!(
                        "{} {}",
                        if active { "Equipped" } else { "Removed" },
                        crate::wardrobe::label(&key)
                    )
                    .into(),
                );
            }
        });
    });

    on!(on_headpat, || {
        with_app(headpat);
    });

    on!(on_tease, || {
        with_app(poke);
    });

    on!(on_hype, || {
        with_app(|app| {
            let stat = memory::adjust_emotion("hype", 6);
            say(
                app,
                "Let's go! Hype level up. High five, partner!",
                "happy",
                vec![format!("hype level {}", stat.level)],
                true,
            );
            sync_studio(app);
        });
    });

    on!(on_groove, || {
        with_app(|app| {
            let stat = memory::adjust_emotion("groove", 6);
            say(
                app,
                "Vibing to the rhythm. Let's get our groove on.",
                "groove",
                vec![format!("groove level {}", stat.level)],
                true,
            );
            sync_studio(app);
        });
    });

    on!(on_reset_emotions, || {
        memory::reset_emotions();
        with_app(|app| {
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_status_text("All emotion EXP reset to baseline.".into());
            }
            sync_studio(app);
        });
    });

    on!(on_trait_changed, |key, value| {
        memory::set_trait(&key, value as i64);
        with_app(|app| {
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_status_text(format!("{key} baseline set to {value}%").into());
            }
        });
    });

    on!(on_add_fact, |text| {
        let (saved, _) = memory::add_fact(&text, "general");
        with_app(move |app| {
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_status_text(
                    if saved {
                        format!("Remembered: {text}")
                    } else {
                        "That one is already in the vault.".to_string()
                    }
                    .into(),
                );
            }
            sync_studio(app);
        });
    });

    on!(on_add_todo, || {
        with_app(|app| {
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_status_text("Add tasks from the chat bar — say “todo add …”.".into());
            }
        });
    });

    on!(on_run_briefing, || {
        with_app(|app| {
            let (msg, logs) = crate::proactive::build_daily_briefing();
            say(app, &msg, "talking", logs, true);
        });
    });

    on!(on_reset_conversation, || {
        crate::brain::brain().reset_memory();
        with_app(|app| {
            if let Some(s) = app.borrow().studio.as_ref() {
                s.set_status_text("Conversation memory cleared.".into());
            }
        });
    });

    on!(on_reset_avatar, || {
        with_app(reset_pose);
    });

    on!(on_open_log, || {
        crate::logger::open_log_file();
    });

    studio.on_quit_app(move || {
        let _ = slint::quit_event_loop();
        std::process::exit(0);
    });

    // A scale change needs one extra pass after the window resizes.
    let _ = (apply_scale, submit_prompt, hex_to_color);
}

fn switch_avatar(app: &AppHandle, mode: &str) {
    let base = crate::assets::start();
    let url = crate::stage::resolve_model_url(base);
    app.borrow().stage.borrow_mut().switch_mode(mode, &url);
    say(
        app,
        &config::flav(&format!("Loaded the {mode} avatar. How does it look?")),
        "happy",
        vec![format!("avatar mode: {mode}")],
        false,
    );
    if let Some(s) = app.borrow().studio.as_ref() {
        s.set_status_text(format!("Avatar switched to {mode}.").into());
    }
}
