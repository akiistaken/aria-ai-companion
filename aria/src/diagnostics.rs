//! Checks the same native services used by the windowless app without starting its UI.

use serde_json::{json, Value};
use std::time::Duration;

fn probe(work: impl FnOnce() -> Value + Send + 'static, seconds: u64) -> Value {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(Duration::from_secs(seconds))
        .unwrap_or_else(
            |_| json!({"error": "Service did not finish before the diagnostic timeout"}),
        )
}

pub(crate) fn audio_output() -> Value {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        IAudioSessionControl2, IAudioSessionManager2, ISimpleAudioVolume,
    };
    use windows::Win32::System::Com::CLSCTX_ALL;

    crate::desktop::com::ensure_mta();
    let read = || -> Option<Value> {
        let enumerator = wasapi::DeviceEnumerator::new().ok()?;
        let collection = enumerator
            .get_device_collection(wasapi::Direction::Render)
            .ok()?;
        let mut devices = Vec::new();
        for index in 0..collection.get_nbr_devices().ok()? {
            let device = collection.get_device_at_index(index).ok()?;
            // All COM interfaces remain on the apartment where they were obtained.
            unsafe {
                let endpoint: IAudioEndpointVolume =
                    device.as_immdevice().Activate(CLSCTX_ALL, None).ok()?;
                let manager: IAudioSessionManager2 =
                    device.as_immdevice().Activate(CLSCTX_ALL, None).ok()?;
                let sessions = manager.GetSessionEnumerator().ok()?;
                let mut own_sessions = Vec::new();
                for index in 0..sessions.GetCount().ok()? {
                    let session = sessions.GetSession(index).ok()?;
                    let control: IAudioSessionControl2 = session.cast().ok()?;
                    if control.GetProcessId().ok()? != std::process::id() {
                        continue;
                    }
                    let volume: ISimpleAudioVolume = session.cast().ok()?;
                    own_sessions.push(json!({"muted": volume.GetMute().ok()?.as_bool(), "volume": volume.GetMasterVolume().ok()?}));
                }
                devices.push(json!({"name": device.get_friendlyname().ok()?,
                    "muted": endpoint.GetMute().ok()?.as_bool(), "volume": endpoint.GetMasterVolumeLevelScalar().ok()?,
                    "aria_sessions": own_sessions}));
            }
        }
        Some(json!(devices))
    };
    read().unwrap_or_else(|| json!({"error": "Could not inspect audio output volume"}))
}

pub fn run() {
    let settings = crate::config::get();
    let activity = probe(|| json!(crate::desktop::presence::refresh()), 10);
    let account = probe(
        || match crate::valorant::account::discover(&crate::config::get()) {
            Ok(Some(account)) => {
                json!({"discovered": true, "source": account.source, "region": account.region})
            }
            Ok(None) => json!({"discovered": false}),
            Err(message) => json!({"error": message}),
        },
        10,
    );
    let local_account = probe(
        || {
            let mut settings = crate::config::get();
            settings.valorant_test_riot_id.clear();
            match crate::valorant::account::discover(&settings) {
                Ok(Some(account)) => json!({"discovered": true, "region": account.region}),
                Ok(None) => json!({"discovered": false}),
                Err(message) => json!({"error": message}),
            }
        },
        10,
    );
    let speech = probe(crate::voice::tts::diagnose, 30);
    let report = json!({
        "tts_enabled": settings.tts_enabled,
        "valorant_enabled": settings.valorant_enabled,
        "game_copilot_enabled": settings.automatic_game_copilot_enabled,
        "activity": activity, "riot_account": account, "riot_local_session": local_account, "speech": speech,
    });
    let path = crate::config::data_subdir("logs").join("diagnostics.json");
    match std::fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()) {
        Ok(()) => log::info!("Diagnostics written to {}", path.display()),
        Err(error) => log::error!("Could not write diagnostics: {error}"),
    }
}
