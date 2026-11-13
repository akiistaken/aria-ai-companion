//! Valorant account/session awareness, visible own-player stats and completed-match analysis.

pub mod account;
pub mod model;
pub mod riot;

use account::Account;
use model::{FrameReading, MatchRecord, Observation, Outcome, Phase};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HistorySummary {
    pub matches: usize,
    pub known_results: usize,
    pub wins: usize,
    pub losses: usize,
    pub draws: usize,
    pub win_percentage: Option<f64>,
    pub kd_sample_matches: usize,
    pub kd: Option<f64>,
}

fn summary(records: &[MatchRecord]) -> HistorySummary {
    let mut result = HistorySummary {
        matches: records.len(),
        ..Default::default()
    };
    let (mut kills, mut deaths) = (0u64, 0u64);
    for record in records {
        match record.outcome {
            Some(Outcome::Win) => result.wins += 1,
            Some(Outcome::Loss) => result.losses += 1,
            Some(Outcome::Draw) => result.draws += 1,
            None => {}
        }
        if let Some((k, d)) = record.stats.kills.zip(record.stats.deaths) {
            result.kd_sample_matches += 1;
            kills += u64::from(k);
            deaths += u64::from(d);
        }
    }
    result.known_results = result.wins + result.losses + result.draws;
    result.win_percentage = (result.known_results > 0)
        .then(|| result.wins as f64 * 100.0 / result.known_results as f64);
    result.kd = (deaths > 0).then(|| kills as f64 / deaths as f64);
    result
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub account: Option<Account>,
    pub account_status: String,
    pub capture_status: String,
    pub game_running: bool,
    pub game_focused: bool,
    pub current: Option<Observation>,
    pub current_stale: bool,
    pub age_seconds: Option<u64>,
    pub recent_matches: Vec<MatchRecord>,
    pub summary: HistorySummary,
    pub recap: Option<String>,
}

struct State {
    generation: u64,
    account: Option<Account>,
    account_status: String,
    capture_status: String,
    active: bool,
    focused: bool,
    current: Option<Observation>,
    history: Vec<MatchRecord>,
    last_account_attempt: Option<Instant>,
    account_poll: Option<Instant>,
    last_capture_attempt: Option<Instant>,
    capture: Option<Instant>,
    refresh_requested: bool,
    result_latched: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            generation: 0,
            account: None,
            account_status: "Waiting for a signed-in Riot Client".into(),
            capture_status: "No game frame observed yet".into(),
            active: false,
            focused: false,
            current: None,
            history: Vec::new(),
            last_account_attempt: None,
            account_poll: None,
            last_capture_attempt: None,
            capture: None,
            refresh_requested: false,
            result_latched: false,
        }
    }
}

impl State {
    fn change_account(&mut self, next: Option<Account>) -> bool {
        let changed = match (&self.account, &next) {
            (Some(a), Some(b)) => !a.same_player(b),
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.generation = self.generation.wrapping_add(1);
            self.current = None;
            self.history.clear();
            self.capture = None;
            self.last_capture_attempt = None;
            self.result_latched = false;
            self.capture_status = "No game frame observed for this account".into();
        }
        self.account = next;
        changed
    }

    fn snapshot(&self, timestamp: u64, interval: u64) -> Snapshot {
        let age = self
            .current
            .as_ref()
            .map(|o| timestamp.saturating_sub(o.observed_at));
        Snapshot {
            account: self.account.clone(),
            account_status: self.account_status.clone(),
            capture_status: self.capture_status.clone(),
            game_running: self.active,
            game_focused: self.focused,
            current: self.current.clone(),
            current_stale: !self.active
                || !self.focused
                || age.is_none_or(|n| n > interval * 2)
                || self
                    .current
                    .as_ref()
                    .is_none_or(|o| o.phase == Phase::Unknown),
            age_seconds: age,
            recent_matches: self.history.clone(),
            summary: summary(&self.history),
            recap: self.history.last().map(model::recap),
        }
    }
}

fn store() -> &'static Mutex<State> {
    static STORE: OnceLock<Mutex<State>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(State::default()))
}

pub fn snapshot() -> Snapshot {
    let interval = crate::config::get().valorant_capture_interval_secs;
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .snapshot(now(), interval)
}

/// Invalidates old workers when setup changes or the user disconnects.
pub fn reset() {
    let mut state = store().lock().unwrap_or_else(|e| e.into_inner());
    let generation = state.generation.wrapping_add(1);
    *state = State {
        generation,
        ..Default::default()
    };
}

pub fn observe_presence(active: bool, focused: bool) {
    let mut state = store().lock().unwrap_or_else(|e| e.into_inner());
    if state.active != active {
        state.generation = state.generation.wrapping_add(1);
        state.current = None;
        state.capture = None;
        state.last_capture_attempt = None;
        state.result_latched = false;
    }
    state.active = active;
    state.focused = focused;
}

pub struct AccountPoll {
    generation: u64,
    started: Instant,
    pub settings: crate::config::Settings,
}

pub fn begin_account_poll() -> Option<AccountPoll> {
    let settings = crate::config::get();
    if !settings.valorant_enabled {
        return None;
    }
    let mut state = store().lock().unwrap_or_else(|e| e.into_inner());
    if state.account_poll.is_some()
        || state
            .last_account_attempt
            .is_some_and(|at| at.elapsed() < Duration::from_secs(15))
    {
        return None;
    }
    let started = Instant::now();
    state.last_account_attempt = Some(started);
    state.account_poll = Some(started);
    Some(AccountPoll {
        generation: state.generation,
        started,
        settings,
    })
}

pub fn finish_account_poll(poll: AccountPoll, result: Result<Option<Account>, &'static str>) {
    let mut state = store().lock().unwrap_or_else(|e| e.into_inner());
    if state.account_poll != Some(poll.started) {
        return;
    }
    state.account_poll = None;
    if state.generation != poll.generation {
        return;
    }
    // On failed discovery, clear identity rather than associating another player's frame with
    // a previously logged-in account. A later successful poll reloads its bounded cache.
    let (account, status) = match result {
        Ok(Some(account)) => {
            let status = format!(
                "{} · {} · {}",
                account.riot_id,
                account.region.as_deref().unwrap_or("region unknown"),
                if account.source == "test_override" {
                    "test override"
                } else {
                    "local Riot session"
                }
            );
            (Some(account), status)
        }
        Ok(None) => (None, "Waiting for a signed-in Riot Client".into()),
        Err(message) => (None, message.into()),
    };
    let changed = state.change_account(account.clone());
    state.account_status = status;
    let generation = state.generation;
    drop(state);
    if changed {
        if let Some(account) = account {
            let records = load_history(&account);
            let mut state = store().lock().unwrap_or_else(|e| e.into_inner());
            if state.generation == generation {
                state.history = records;
            }
        }
    }
}

pub fn request_refresh() {
    let mut state = store().lock().unwrap_or_else(|e| e.into_inner());
    state.refresh_requested = true;
    state.capture_status = "Return to Valorant to capture the visible scoreboard or results".into();
    state.last_account_attempt = None;
}

#[derive(Clone)]
pub struct CaptureTicket {
    generation: u64,
    started: Instant,
    pub observed_at: u64,
    pub account: Account,
}

pub fn begin_capture() -> Option<CaptureTicket> {
    let settings = crate::config::get();
    if !settings.valorant_enabled
        || !settings.automatic_game_copilot_enabled
        || !crate::ai::client::available()
    {
        return None;
    }
    let mut state = store().lock().unwrap_or_else(|e| e.into_inner());
    if !state.active || !state.focused || state.capture.is_some() {
        return None;
    }
    let delay = if state.refresh_requested {
        10
    } else {
        settings.valorant_capture_interval_secs
    };
    if state
        .last_capture_attempt
        .is_some_and(|at| at.elapsed() < Duration::from_secs(delay))
    {
        return None;
    }
    let account = state.account.clone()?;
    let started = Instant::now();
    state.capture = Some(started);
    state.last_capture_attempt = Some(started);
    state.refresh_requested = false;
    state.capture_status = "Reading the visible Valorant frame".into();
    Some(CaptureTicket {
        generation: state.generation,
        started,
        observed_at: now(),
        account,
    })
}

/// Returns a new completed record once per results-screen visit. No live tactical commentary.
pub fn finish_capture(
    ticket: CaptureTicket,
    reading: Result<FrameReading, &'static str>,
) -> Option<MatchRecord> {
    let enabled = crate::config::get().valorant_enabled;
    let mut state = store().lock().unwrap_or_else(|e| e.into_inner());
    if state.capture != Some(ticket.started) {
        return None;
    }
    state.capture = None;
    if !enabled || state.generation != ticket.generation {
        return None;
    }
    if !state.focused || ticket.started.elapsed() > Duration::from_secs(25) {
        state.current = None;
        state.capture_status = "Frame discarded after a focus change or delay".into();
        return None;
    }
    let reading = match reading {
        Ok(reading) => reading,
        Err(message) => {
            state.current = None;
            state.capture_status = message.into();
            return None;
        }
    };
    let observation = reading.observation(&ticket.account.riot_id, ticket.observed_at);
    state.capture_status = match observation.phase {
        Phase::Unknown => "Frame unclear; statistics remain unknown",
        Phase::Lobby => "Lobby observed; no current-match statistics",
        Phase::InMatch => "Visible match observed; missing fields remain unknown",
        Phase::Results => "Results screen observed",
    }
    .into();
    if matches!(observation.phase, Phase::Lobby | Phase::InMatch) {
        state.result_latched = false;
    }
    let completed = if !state.result_latched {
        observation.completed(format!(
            "screen-{}-{}",
            ticket.observed_at, ticket.generation
        ))
    } else {
        None
    };
    state.current = Some(observation);
    if let Some(record) = &completed {
        state.result_latched = true;
        append_record(&mut state.history, record.clone());
    }
    let records = completed.as_ref().map(|_| state.history.clone());
    drop(state);
    if let Some(records) = records {
        save_history(&ticket.account, &records);
    }
    completed
}

fn append_record(records: &mut Vec<MatchRecord>, record: MatchRecord) {
    if let Some(existing) = records
        .iter_mut()
        .find(|r| r.id == record.id && r.source == record.source)
    {
        *existing = record;
    } else {
        records.push(record);
    }
    if records.len() > 20 {
        records.drain(..records.len() - 20);
    }
}

#[derive(Serialize, Deserialize)]
struct History {
    account: Account,
    records: Vec<MatchRecord>,
}

fn history_path(account: &Account) -> std::path::PathBuf {
    crate::config::data_subdir("valorant").join(format!("{}.json", account.cache_key()))
}

fn load_history(account: &Account) -> Vec<MatchRecord> {
    let read = || -> Option<History> {
        let file = std::fs::File::open(history_path(account)).ok()?;
        let mut raw = String::new();
        file.take(8 * 1024 * 1024).read_to_string(&mut raw).ok()?;
        serde_json::from_str(&raw).ok()
    };
    read()
        .filter(|h| h.account.same_player(account))
        .map(|h| {
            h.records
                .into_iter()
                .rev()
                .take(20)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        })
        .unwrap_or_default()
}

fn save_history(account: &Account, records: &[MatchRecord]) {
    // A file import and a results-screen worker can finish together. Serialise disk writes
    // and merge by record ID so an older snapshot cannot erase a newer completed match.
    static SAVING: Mutex<()> = Mutex::new(());
    let _saving = SAVING.lock().unwrap_or_else(|e| e.into_inner());
    let mut merged = load_history(account);
    for record in records {
        if let Some(existing) = merged
            .iter_mut()
            .find(|r| r.id == record.id && r.source == record.source)
        {
            *existing = record.clone();
        } else {
            merged.push(record.clone());
        }
    }
    merged.sort_by_key(|record| record.recorded_at);
    if merged.len() > 20 {
        merged.drain(..merged.len() - 20);
    }
    let history = History {
        account: account.clone(),
        records: merged,
    };
    let path = history_path(account);
    let pending = path.with_extension("pending");
    let written = serde_json::to_vec_pretty(&history).ok().is_some_and(|raw| {
        std::fs::write(&pending, raw).is_ok() && std::fs::rename(&pending, &path).is_ok()
    });
    if !written {
        log::warn!("could not save Valorant own-player match history");
    }
}

pub fn import_match(raw: &str) -> Result<MatchRecord, &'static str> {
    if !crate::config::get().valorant_enabled {
        return Err("Enable Valorant awareness before importing a match");
    }
    let state = store().lock().unwrap_or_else(|e| e.into_inner());
    let account = state
        .account
        .clone()
        .ok_or("Sign in to Riot Client or set a test account first")?;
    let generation = state.generation;
    drop(state);
    let record = riot::parse_match(raw, &account, now())?;
    let mut state = store().lock().unwrap_or_else(|e| e.into_inner());
    if state.generation != generation {
        return Err("Account changed during match import; try again");
    }
    append_record(&mut state.history, record.clone());
    let history = state.history.clone();
    drop(state);
    save_history(&account, &history);
    Ok(record)
}

pub fn ui_current(data: &Snapshot) -> String {
    let Some(observation) = &data.current else {
        return data.capture_status.clone();
    };
    let mut fields = vec![format!("{:?}", observation.phase)];
    if let Some(map) = &observation.map {
        fields.push(map.clone());
    }
    if let Some(agent) = &observation.agent {
        fields.push(agent.clone());
    }
    if let Some([own, other]) = observation.team_score {
        fields.push(format!("Score {own}-{other}"));
    }
    let value = |v: Option<u32>| v.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
    fields.push(format!(
        "Your K/D/A {}/{}/{}",
        value(observation.stats.kills),
        value(observation.stats.deaths),
        value(observation.stats.assists)
    ));
    fields.push(format!(
        "visible screen · {}s ago{}",
        data.age_seconds.unwrap_or(0),
        if data.current_stale { " · stale" } else { "" }
    ));
    fields.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::Stats;

    fn account(name: &str) -> Account {
        Account {
            riot_id: name.into(),
            puuid: None,
            region: None,
            source: "fixture".into(),
        }
    }

    fn record(id: &str) -> MatchRecord {
        MatchRecord {
            id: id.into(),
            account: "one#tag".into(),
            source: "fixture".into(),
            recorded_at: 1,
            map: None,
            agent: None,
            team_score: None,
            outcome: None,
            stats: Stats::default(),
        }
    }

    #[test]
    fn account_switch_clears_frames_and_history_and_invalidates_workers() {
        let mut state = State::default();
        state.change_account(Some(account("one#tag")));
        state.history.push(record("old"));
        let generation = state.generation;
        state.change_account(Some(account("two#tag")));
        assert!(state.history.is_empty());
        assert!(state.current.is_none());
        assert_ne!(state.generation, generation);
    }

    #[test]
    fn freshness_expires_and_alt_tab_marks_data_stale() {
        let mut state = State {
            active: true,
            focused: true,
            ..Default::default()
        };
        state.current = Some(
            FrameReading {
                phase: Phase::InMatch,
                phase_confidence: 0.99,
                ..Default::default()
            }
            .observation("one#tag", 100),
        );
        assert!(!state.snapshot(110, 60).current_stale);
        assert!(state.snapshot(221, 60).current_stale);
        state.focused = false;
        assert!(state.snapshot(110, 60).current_stale);
    }

    #[test]
    fn imports_are_deduplicated_and_history_is_bounded() {
        let mut history = Vec::new();
        for n in 0..25 {
            append_record(&mut history, record(&n.to_string()));
        }
        assert_eq!(history.len(), 20);
        assert_eq!(history[0].id, "5");
        append_record(&mut history, record("24"));
        assert_eq!(history.len(), 20);
    }

    #[test]
    fn win_percentage_excludes_unknown_results_and_kd_uses_totals() {
        let mut a = record("a");
        a.outcome = Some(Outcome::Win);
        a.stats.kills = Some(10);
        a.stats.deaths = Some(2);
        let mut b = record("b");
        b.stats.kills = Some(10);
        b.stats.deaths = Some(8);
        let data = summary(&[a, b]);
        assert_eq!(data.win_percentage, Some(100.0));
        assert_eq!(data.known_results, 1);
        assert_eq!(data.kd, Some(2.0));
    }
}
