//! Normalise a completed VAL-MATCH-V1 response, retaining only the signed-in player's stats.
//! This parser is also the adapter boundary for future approved Riot/RSO backend access.

use super::account::Account;
use super::model::{MatchRecord, Outcome, Stats};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiotMatch {
    match_info: MatchInfo,
    players: Vec<Player>,
    #[serde(default)]
    teams: Vec<Team>,
    #[serde(default)]
    round_results: Vec<Round>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MatchInfo {
    match_id: String,
    map_id: String,
    is_completed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Player {
    puuid: String,
    #[serde(default)]
    game_name: String,
    #[serde(default)]
    tag_line: String,
    team_id: String,
    #[serde(default)]
    character_id: String,
    stats: Option<PlayerStats>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerStats {
    kills: u32,
    deaths: u32,
    assists: u32,
    score: u32,
    rounds_played: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Team {
    team_id: String,
    #[serde(default)]
    won: bool,
    rounds_won: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Round {
    round_num: u32,
    #[serde(default)]
    player_stats: Vec<RoundPlayer>,
}

#[derive(Deserialize)]
struct RoundPlayer {
    puuid: String,
    // None distinguishes missing damage data from a legitimate empty list (zero hits).
    damage: Option<Vec<Damage>>,
}

#[derive(Deserialize)]
struct Damage {
    damage: u32,
    headshots: u32,
    bodyshots: u32,
    legshots: u32,
}

fn map_name(path: &str) -> Option<String> {
    let asset = path.rsplit('/').next()?.trim();
    if asset.is_empty() || asset.chars().count() > 64 {
        return None;
    }
    let name = match asset {
        "Duality" => "Bind",
        "Bonsai" => "Split",
        "Triad" => "Haven",
        "Port" => "Icebox",
        "Foxtrot" => "Breeze",
        "Canyon" => "Fracture",
        "Pitt" => "Pearl",
        "Jam" => "Lotus",
        "Juliett" => "Sunset",
        "Infinity" => "Abyss",
        other => other,
    };
    Some(name.into())
}

pub fn parse_match(raw: &str, account: &Account, at: u64) -> Result<MatchRecord, &'static str> {
    if raw.len() > 8 * 1024 * 1024 {
        return Err("Match JSON exceeds 8 MB");
    }
    let data: RiotMatch =
        serde_json::from_str(raw).map_err(|_| "Expected a VAL-MATCH-V1 match response")?;
    if !data.match_info.is_completed {
        return Err("Only completed Riot matches can be imported");
    }
    if data.match_info.match_id.is_empty() || data.match_info.match_id.len() > 128 {
        return Err("Match response has an invalid match ID");
    }
    let own: Vec<_> = data
        .players
        .iter()
        .filter(|p| {
            if let Some(puuid) = &account.puuid {
                return p.puuid.eq_ignore_ascii_case(puuid);
            }
            format!("{}#{}", p.game_name, p.tag_line).eq_ignore_ascii_case(&account.riot_id)
        })
        .collect();
    if own.len() != 1 {
        return Err("Could not identify the signed-in player in this match");
    }
    let own = own[0];
    let stats = own
        .stats
        .as_ref()
        .ok_or("This match has no own-player statistics")?;
    if stats.rounds_played > 1000
        || [stats.kills, stats.deaths, stats.assists]
            .iter()
            .any(|n| *n > 10000)
    {
        return Err("Match response has invalid player statistics");
    }
    let team = data.teams.iter().find(|t| t.team_id == own.team_id);
    let other = (data.teams.len() == 2)
        .then(|| data.teams.iter().find(|t| t.team_id != own.team_id))
        .flatten();
    let team_score = team.zip(other).map(|(a, b)| [a.rounds_won, b.rounds_won]);
    let outcome = team.zip(other).map(|(a, b)| {
        if a.won {
            Outcome::Win
        } else if b.won {
            Outcome::Loss
        } else {
            Outcome::Draw
        }
    });
    let mut result = Stats {
        kills: Some(stats.kills),
        deaths: Some(stats.deaths),
        assists: Some(stats.assists),
        rounds_played: Some(stats.rounds_played),
        total_combat_score: Some(f64::from(stats.score)),
        ..Default::default()
    };
    // Damage/hit metrics require complete, unique own-player round data. Partial JSON must
    // not silently produce a misleading low ADR or zero headshot percentage.
    let mut seen = std::collections::HashSet::new();
    let mut totals = [0u64; 4];
    let complete = stats.rounds_played > 0
        && data.round_results.len() == stats.rounds_played as usize
        && data.round_results.iter().all(|round| {
            if !seen.insert(round.round_num) {
                return false;
            }
            let own_round: Vec<_> = round
                .player_stats
                .iter()
                .filter(|p| p.puuid == own.puuid)
                .collect();
            if own_round.len() != 1 {
                return false;
            }
            let Some(damage) = &own_round[0].damage else {
                return false;
            };
            for hit in damage {
                for (total, n) in
                    totals
                        .iter_mut()
                        .zip([hit.damage, hit.headshots, hit.bodyshots, hit.legshots])
                {
                    *total += u64::from(n);
                }
            }
            true
        });
    if complete && totals[1..].iter().all(|n| *n <= u64::from(u32::MAX)) {
        result.total_damage = Some(totals[0] as f64);
        result.headshots = Some(totals[1] as u32);
        result.bodyshots = Some(totals[2] as u32);
        result.legshots = Some(totals[3] as u32);
    }
    Ok(MatchRecord {
        id: data.match_info.match_id,
        account: account.riot_id.clone(),
        source: "riot_match_import".into(),
        recorded_at: at,
        map: map_name(&data.match_info.map_id),
        agent: (!own.character_id.is_empty()).then(|| own.character_id.clone()),
        team_score,
        outcome,
        stats: result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account() -> Account {
        Account {
            riot_id: "Akii#myr".into(),
            puuid: Some("own".into()),
            region: Some("ap".into()),
            source: "fixture".into(),
        }
    }

    fn fixture() -> serde_json::Value {
        json!({"matchInfo":{"matchId":"completed-match","mapId":"/Game/Maps/Ascent/Ascent","isCompleted":true},
            "players":[{"puuid":"other","teamId":"Blue","stats":{"kills":99,"deaths":1,"assists":0,"score":9999,"roundsPlayed":2}},
                {"puuid":"own","gameName":"Akii","tagLine":"myr","teamId":"Red","stats":{"kills":4,"deaths":2,"assists":1,"score":400,"roundsPlayed":2}}],
            "teams":[{"teamId":"Red","won":true,"roundsWon":2},{"teamId":"Blue","won":false,"roundsWon":0}],
            "roundResults":[{"roundNum":0,"playerStats":[{"puuid":"own","damage":[{"damage":100,"headshots":1,"bodyshots":2,"legshots":0}]}]},
                {"roundNum":1,"playerStats":[{"puuid":"own","damage":[{"damage":200,"headshots":1,"bodyshots":3,"legshots":1}]}]}]})
    }

    #[test]
    fn importer_selects_own_puuid_and_computes_exact_metrics() {
        let record = parse_match(&fixture().to_string(), &account(), 1).unwrap();
        assert_eq!(record.stats.kills, Some(4));
        assert_eq!(record.stats.metrics().acs, Some(200.0));
        assert_eq!(record.stats.metrics().adr, Some(150.0));
        assert_eq!(record.stats.metrics().headshot_percentage, Some(25.0));
        assert_eq!(record.team_score, Some([2, 0]));
        assert_eq!(record.outcome, Some(Outcome::Win));
    }

    #[test]
    fn incomplete_or_duplicate_rounds_do_not_create_damage_metrics() {
        let mut raw = fixture();
        raw["roundResults"][1]["roundNum"] = json!(0);
        assert!(parse_match(&raw.to_string(), &account(), 1)
            .unwrap()
            .stats
            .metrics()
            .adr
            .is_none());
        raw["roundResults"] = json!([]);
        let record = parse_match(&raw.to_string(), &account(), 1).unwrap();
        assert!(record.stats.metrics().adr.is_none());
        assert_eq!(record.stats.metrics().acs, Some(200.0));
    }

    #[test]
    fn wrong_account_unfinished_match_and_ambiguous_name_are_rejected() {
        let mut raw = fixture();
        raw["matchInfo"]["isCompleted"] = json!(false);
        assert!(parse_match(&raw.to_string(), &account(), 1).is_err());
        raw["matchInfo"]["isCompleted"] = json!(true);
        let mut who = account();
        who.puuid = Some("someone-else".into());
        assert!(parse_match(&raw.to_string(), &who, 1).is_err());
        who.puuid = None;
        assert!(parse_match(&raw.to_string(), &who, 1).is_ok());
        let mut duplicate = raw["players"][1].clone();
        duplicate["puuid"] = json!("duplicate");
        raw["players"].as_array_mut().unwrap().push(duplicate);
        assert!(parse_match(&raw.to_string(), &who, 1).is_err());
    }
}
