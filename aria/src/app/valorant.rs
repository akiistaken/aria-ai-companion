//! UI-thread scheduling of independent account and frame workers.

use super::*;

pub(crate) fn sync(app: &AppHandle) {
    if !config::get().valorant_enabled {
        let borrowed = app.borrow();
        if let Some(studio) = &borrowed.studio {
            studio.set_valorant_account("Valorant awareness is off".into());
            studio.set_valorant_status("Enable awareness to discover the signed-in account".into());
            studio.set_valorant_current("No current match observation".into());
            studio.set_valorant_history("".into());
            studio.set_valorant_recap("".into());
        }
        return;
    }
    let data = crate::valorant::snapshot();
    let borrowed = app.borrow();
    let Some(studio) = &borrowed.studio else {
        return;
    };
    studio.set_valorant_account(data.account_status.clone().into());
    studio.set_valorant_status(data.capture_status.clone().into());
    studio.set_valorant_current(crate::valorant::ui_current(&data).into());
    studio.set_valorant_recap(
        data.recap
            .unwrap_or_else(|| {
                "A recap appears after a readable results screen or Riot match import.".into()
            })
            .into(),
    );
    let history = if data.summary.matches == 0 {
        "No completed matches recorded for this account".into()
    } else {
        format!(
            "{} recorded matches · {} known results · {} wins / {} losses / {} draws{}",
            data.summary.matches,
            data.summary.known_results,
            data.summary.wins,
            data.summary.losses,
            data.summary.draws,
            data.summary
                .kd
                .map(|kd| format!(
                    " · K/D {kd:.2} across {} matches",
                    data.summary.kd_sample_matches
                ))
                .unwrap_or_default()
        )
    };
    studio.set_valorant_history(history.into());
}

pub(crate) fn tick(app: &AppHandle, activity: &crate::desktop::presence::Activity) {
    let settings = config::get();
    if !settings.valorant_enabled {
        return;
    }
    let focused = activity
        .foreground_game
        .as_ref()
        .is_some_and(|g| g.name == "Valorant");
    let active = focused || activity.running_games.iter().any(|g| g.name == "Valorant");
    crate::valorant::observe_presence(active, focused);
    if let Some(poll) = crate::valorant::begin_account_poll() {
        std::thread::spawn(move || {
            let account = crate::valorant::account::discover(&poll.settings);
            crate::valorant::finish_account_poll(poll, account);
            post(sync);
        });
    }
    sync(app);
    let a = app.borrow();
    let occupied = a.busy
        || a.dragging
        || a.reaction_pending
        || crate::voice::is_recording()
        || crate::voice::is_speaking();
    let generation = a.speech_generation;
    drop(a);
    if occupied {
        return;
    }
    if let Some(ticket) = crate::valorant::begin_capture() {
        std::thread::spawn(move || {
            let reading = crate::ai::tasks::valorant_frame(&ticket.account);
            let record = crate::valorant::finish_capture(ticket, reading);
            post(move |app| {
                sync(app);
                let Some(record) = record else { return };
                let a = app.borrow();
                let may_speak = a.speech_generation == generation
                    && !a.busy
                    && !a.dragging
                    && !a.reaction_pending
                    && a.foreground_game
                        .as_ref()
                        .is_some_and(|g| g.name == "Valorant")
                    && config::get().valorant_enabled
                    && config::get().automatic_game_copilot_enabled
                    && !crate::voice::is_recording()
                    && !crate::voice::is_speaking();
                drop(a);
                if may_speak {
                    let stats = &record.stats;
                    let line = format!(
                        "Match finished: {}/{}/{}. Your recap is ready in Presence & gaming.",
                        stats.kills.unwrap_or(0),
                        stats.deaths.unwrap_or(0),
                        stats.assists.unwrap_or(0)
                    );
                    say(
                        app,
                        &line,
                        "happy",
                        vec!["Valorant post-match recap".into()],
                        true,
                    );
                }
            });
        });
    }
}

pub(crate) fn import_file() {
    // Native file picker and file parsing stay off the Slint event loop.
    std::thread::spawn(|| {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Riot match JSON", &["json"])
            .set_title("Import a completed VAL-MATCH-V1 response")
            .pick_file()
        else {
            return;
        };
        use std::io::Read;
        let result = (|| {
            let file = std::fs::File::open(path).map_err(|_| "Could not open the match file")?;
            let mut raw = String::new();
            file.take(8 * 1024 * 1024 + 1)
                .read_to_string(&mut raw)
                .map_err(|_| "Could not read the match file")?;
            crate::valorant::import_match(&raw)
        })();
        post(move |app| {
            sync(app);
            if let Some(studio) = &app.borrow().studio {
                studio.set_valorant_status(match result {
                    Ok(_) => "Completed Riot match imported".into(),
                    Err(message) => message.into(),
                });
            }
        });
    });
}
