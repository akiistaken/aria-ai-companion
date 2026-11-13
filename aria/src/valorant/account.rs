//! Read-only account discovery from the Riot Client's loopback service.
//! These local endpoints are community documented, not the production Riot API.
//! Lockfile credentials never leave this module, reach a proxy, or enter persisted state.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub riot_id: String,
    pub puuid: Option<String>,
    pub region: Option<String>,
    pub source: String,
}

impl Account {
    pub fn same_player(&self, other: &Self) -> bool {
        match (&self.puuid, &other.puuid) {
            (Some(a), Some(b)) => a == b,
            _ => self.riot_id.eq_ignore_ascii_case(&other.riot_id),
        }
    }

    pub fn cache_key(&self) -> String {
        // UUIDs are validated on discovery. Test overrides use a filesystem-safe hash.
        if let Some(puuid) = &self.puuid {
            return puuid.clone();
        }
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.riot_id.to_lowercase().hash(&mut hash);
        format!("test-{:016x}", hash.finish())
    }
}

pub fn valid_riot_id(id: &str) -> bool {
    let Some((name, tag)) = id.split_once('#') else {
        return false;
    };
    !name.trim().is_empty()
        && !tag.trim().is_empty()
        && !tag.contains('#')
        && id.chars().count() <= 64
        && !id.chars().any(char::is_control)
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

// Deliberately no Debug/Serialize: this contains the ephemeral local password.
struct Lockfile {
    pid: u32,
    port: u16,
    password: String,
}

impl Lockfile {
    fn parse(raw: &str) -> Option<Self> {
        let fields: Vec<_> = raw.trim().split(':').collect();
        if fields.len() != 5
            || fields[4] != "https"
            || fields[3].is_empty()
            || fields[3].len() > 256
            || fields[3].chars().any(char::is_control)
        {
            return None;
        }
        let pid = fields[1].parse().ok()?;
        let port = fields[2].parse().ok()?;
        if pid == 0 || port == 0 {
            return None;
        }
        Some(Self {
            pid,
            port,
            password: fields[3].into(),
        })
    }

    fn get(&self, agent: &ureq::Agent, path: &str) -> Result<Value, &'static str> {
        // Both host and paths are fixed by this module, never taken from API responses.
        let url = format!("https://127.0.0.1:{}{path}", self.port);
        let auth = format!(
            "Basic {}",
            crate::util::base64(format!("riot:{}", self.password).as_bytes())
        );
        let mut response = agent
            .get(&url)
            .header("Authorization", &auth)
            .call()
            .map_err(|_| "Riot Client session unavailable")?;
        match response.status().as_u16() {
            200 => {}
            401 | 403 => return Err("Riot Client local authentication expired; sign in again"),
            404 => return Err("Riot Client session endpoint is not available"),
            _ => return Err("Riot Client session unavailable"),
        }
        let mut body = String::new();
        response
            .body_mut()
            .as_reader()
            .take(256 * 1024)
            .read_to_string(&mut body)
            .map_err(|_| "Could not read the Riot Client session")?;
        serde_json::from_str(&body).map_err(|_| "Unrecognised Riot Client session response")
    }
}

pub fn account_from_session(value: &Value) -> Option<Account> {
    if value.get("loaded")?.as_bool()? != true {
        return None;
    }
    let name = value.get("game_name")?.as_str()?.trim();
    let tag = value.get("game_tag")?.as_str()?.trim();
    let puuid = value.get("puuid")?.as_str()?;
    let riot_id = format!("{name}#{tag}");
    if !valid_riot_id(&riot_id) || !valid_uuid(puuid) {
        return None;
    }
    Some(Account {
        riot_id,
        puuid: Some(puuid.to_lowercase()),
        region: None,
        source: "local_riot_client".into(),
    })
}

pub fn account_from_userinfo(value: &Value) -> Option<Account> {
    // The client wraps JSON in a string. Extract only the alias and PUUID, discarding all
    // other account claims. An empty userInfo is a logged-out/uninitialised session.
    let info: Value = serde_json::from_str(value.get("userInfo")?.as_str()?).ok()?;
    let riot_id = format!(
        "{}#{}",
        info["acct"]["game_name"].as_str()?.trim(),
        info["acct"]["tag_line"].as_str()?.trim()
    );
    let puuid = info["sub"].as_str()?;
    if !valid_riot_id(&riot_id) || !valid_uuid(puuid) {
        return None;
    }
    Some(Account {
        riot_id,
        puuid: Some(puuid.to_lowercase()),
        region: None,
        source: "local_riot_client".into(),
    })
}

fn deployment(sessions: &Value) -> Option<String> {
    sessions
        .as_object()?
        .values()
        .filter(|v| v["productId"] == "valorant")
        .filter_map(|v| v["launchConfiguration"]["arguments"].as_array())
        .flatten()
        .filter_map(Value::as_str)
        .find_map(|arg| {
            let region = arg.strip_prefix("-ares-deployment=")?.to_lowercase();
            ["ap", "na", "eu", "kr", "br", "latam"]
                .contains(&region.as_str())
                .then_some(region)
        })
}

pub fn discover(settings: &crate::config::Settings) -> Result<Option<Account>, &'static str> {
    if !settings.valorant_enabled {
        return Ok(None);
    }
    if !settings.valorant_test_riot_id.is_empty() {
        if !valid_riot_id(&settings.valorant_test_riot_id) {
            return Err("Test Riot ID must be name#tag");
        }
        return Ok(Some(Account {
            riot_id: settings.valorant_test_riot_id.clone(),
            puuid: None,
            region: Some(settings.valorant_test_region.clone()),
            source: "test_override".into(),
        }));
    }
    let Some(local) = std::env::var_os("LOCALAPPDATA") else {
        return Ok(None);
    };
    let path = std::path::PathBuf::from(local).join("Riot Games/Riot Client/Config/lockfile");
    let Ok(file) = std::fs::File::open(path) else {
        return Ok(None);
    };
    let mut raw = String::new();
    file.take(2048)
        .read_to_string(&mut raw)
        .map_err(|_| "Could not read the Riot Client lockfile")?;
    let lock = Lockfile::parse(&raw).ok_or("Unrecognised Riot Client lockfile")?;
    if crate::desktop::win32::process_name(lock.pid) != "riotclientservices.exe" {
        return Err("Riot Client process not available");
    }
    // Riot's local service uses a self-signed certificate. This exception is scoped to this
    // dedicated, fixed-loopback agent; normal app HTTPS clients keep certificate verification.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(3)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .disable_verification(true)
                .build(),
        )
        .build()
        .into();
    // userinfo is available in the Riot launcher before Valorant starts. Chat/session is
    // absent in some launcher builds, so it is only a compatibility fallback.
    let detected = match lock.get(&agent, "/rso-auth/v1/authorization/userinfo") {
        Ok(info) => account_from_userinfo(&info),
        Err("Riot Client session endpoint is not available") => {
            account_from_session(&lock.get(&agent, "/chat/v1/session")?)
        }
        Err(message) => return Err(message),
    };
    let Some(mut account) = detected else {
        return Ok(None);
    };
    // Locale/chat region is not a reliable Valorant deployment. Read only the relevant launch
    // argument; no launch arguments, auth tokens, friends or opponent identities are retained.
    account.region = lock
        .get(&agent, "/product-session/v1/external-sessions")
        .ok()
        .and_then(|sessions| deployment(&sessions));
    Ok(Some(account))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn login_requires_a_loaded_session_and_valid_identity() {
        let mut value = json!({"loaded":true,"game_name":"Akii","game_tag":"myr",
            "puuid":"12345678-1234-1234-1234-123456789abc"});
        let account = account_from_session(&value).unwrap();
        assert_eq!(account.riot_id, "Akii#myr");
        value["loaded"] = json!(false);
        assert!(account_from_session(&value).is_none());
        value["loaded"] = json!(true);
        value["puuid"] = json!("../../another-account");
        assert!(account_from_session(&value).is_none());
    }

    #[test]
    fn lockfile_rejects_non_https_and_invalid_ports() {
        assert!(Lockfile::parse("RiotClient:2:2999:fixture-password:https").is_some());
        assert!(Lockfile::parse("RiotClient:2:0:fixture-password:https").is_none());
        assert!(Lockfile::parse("RiotClient:2:2999:fixture-password:http").is_none());
    }

    #[test]
    fn launcher_userinfo_detects_login_and_rejects_logout() {
        let value = json!({"userInfo":json!({"sub":"12345678-1234-1234-1234-123456789abc",
            "acct":{"game_name":"Akii","tag_line":"myr"}}).to_string()});
        assert_eq!(account_from_userinfo(&value).unwrap().riot_id, "Akii#myr");
        assert!(account_from_userinfo(&json!({"userInfo":""})).is_none());
        assert!(account_from_userinfo(&json!({"userInfo":"{}"})).is_none());
    }

    #[test]
    fn region_comes_from_valorant_deployment_only() {
        let sessions = json!({"riot":{"productId":"riot_client","launchConfiguration":{"arguments":["-ares-deployment=eu"]}},
            "game":{"productId":"valorant","launchConfiguration":{"arguments":["-ares-deployment=ap"]}}});
        assert_eq!(deployment(&sessions).as_deref(), Some("ap"));
    }

    #[test]
    #[ignore = "reads the currently signed-in local Riot Client session"]
    fn live_account_discovery() {
        let settings = crate::config::Settings {
            valorant_enabled: true,
            ..Default::default()
        };
        match discover(&settings) {
            Ok(Some(account)) => {
                assert!(valid_riot_id(&account.riot_id));
                assert!(account.puuid.as_deref().is_some_and(valid_uuid));
                println!(
                    "Detected Riot account: {} · region: {} · source: {}",
                    account.riot_id,
                    account
                        .region
                        .as_deref()
                        .unwrap_or("unknown until Valorant starts"),
                    account.source
                );
            }
            Ok(None) => println!("No loaded local Riot session; waiting for login"),
            Err(message) => panic!("Local Riot discovery failed: {message}"),
        }
    }
}
