//! Direct-Android update discovery. APKs are never fetched or installed here;
//! the only update action is handing a validated HTTPS URL to the browser.
#![cfg_attr(
    not(all(target_os = "android", feature = "direct-updates")),
    allow(dead_code)
)]
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const LIMIT: usize = 64 * 1024;
const INTERVAL: u64 = 24 * 60 * 60;
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Deserialize)]
pub struct Identity {
    pub package_id: String,
    pub version: String,
    pub version_code: u64,
    pub sdk: u32,
    pub abis: Vec<String>,
    pub certificate_sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Release {
    pub version: String,
    pub version_code: u64,
    pub abi: String,
    pub min_sdk: u32,
    pub url: String,
    pub sha256: String,
    pub certificate_sha256: String,
    pub size_bytes: u64,
    #[serde(default)]
    pub notes: String,
}
#[derive(Deserialize)]
struct Feed {
    schema_version: u32,
    package_id: String,
    channel: String,
    releases: Vec<Release>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
struct Preferences {
    automatic: bool,
    feed_url: Option<String>,
    last_checked: Option<u64>,
    retry_at: Option<u64>,
    error: Option<String>,
    release: Option<Release>,
    dismissed_version: Option<u64>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            automatic: true,
            feed_url: None,
            last_checked: None,
            retry_at: None,
            error: None,
            release: None,
            dismissed_version: None,
        }
    }
}
#[derive(Clone, Serialize)]
pub struct Status {
    pub enabled: bool,
    pub automatic: bool,
    pub feed_url: Option<String>,
    pub current_version: String,
    pub current_version_code: u64,
    pub last_checked: Option<u64>,
    pub retry_at: Option<u64>,
    pub error: Option<String>,
    pub release: Option<Release>,
    pub dismissed_version: Option<u64>,
}
impl Status {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            automatic: false,
            feed_url: None,
            current_version: String::new(),
            current_version_code: 0,
            last_checked: None,
            retry_at: None,
            error: None,
            release: None,
            dismissed_version: None,
        }
    }
}
pub struct Updater {
    path: PathBuf,
    default_feed: Option<String>,
    preferences: Mutex<Preferences>,
    client: Client,
}

pub fn https_url(value: &str) -> Result<Url, String> {
    if value.len() > 2048 || value.chars().any(char::is_control) {
        return Err("Invalid update address".into());
    }
    let url = Url::parse(value.trim()).map_err(|_| "Enter a complete HTTPS update address")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("Update addresses must use HTTPS without credentials or fragments".into());
    }
    Ok(url)
}
fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn select(bytes: &[u8], identity: &Identity) -> Result<Option<Release>, String> {
    let feed: Feed = serde_json::from_slice(bytes)
        .map_err(|_| "The update server did not return a valid release feed")?;
    if feed.schema_version != 1
        || feed.package_id != identity.package_id
        || feed.channel != "direct"
        || feed.releases.len() > 100
    {
        return Err(
            "The release feed is for a different app, channel, or unsupported format".into(),
        );
    }
    let release = feed
        .releases
        .into_iter()
        .filter(|r| {
            r.version_code > identity.version_code
                && r.min_sdk <= identity.sdk
                && identity.abis.contains(&r.abi)
        })
        .max_by_key(|r| r.version_code);
    if let Some(r) = &release {
        https_url(&r.url)?;
        if r.version.trim().is_empty()
            || r.version.len() > 64
            || r.notes.len() > 8000
            || r.version_code > i32::MAX as u64
            || r.size_bytes == 0
            || !digest(&r.sha256)
            || !digest(&r.certificate_sha256)
        {
            return Err("The release metadata is incomplete or invalid".into());
        }
        if !r
            .certificate_sha256
            .eq_ignore_ascii_case(&identity.certificate_sha256)
        {
            return Err(
                "This release uses a different signing key and cannot update this installation"
                    .into(),
            );
        }
    }
    Ok(release)
}
impl Updater {
    pub fn new(path: PathBuf, default_feed: Option<&str>) -> Self {
        let default_feed = default_feed
            .filter(|url| !url.is_empty())
            .map(str::to_owned);
        let mut preferences = match std::fs::metadata(&path) {
            Ok(m) if m.len() <= LIMIT as u64 => std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default(),
            _ => Preferences::default(),
        };
        for source in [preferences.feed_url.as_deref(), default_feed.as_deref()]
            .into_iter()
            .flatten()
        {
            if let Err(error) = https_url(source) {
                preferences.error = Some(error);
            }
        }
        Self {
            path,
            default_feed,
            preferences: Mutex::new(preferences),
            client: Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!(
                    "Zimsmith/",
                    env!("CARGO_PKG_VERSION"),
                    " update-check"
                ))
                .build()
                .expect("HTTP client"),
        }
    }
    fn source(&self, p: &Preferences) -> Option<String> {
        p.feed_url.clone().or_else(|| self.default_feed.clone())
    }
    fn status(&self, p: &Preferences, identity: &Identity) -> Status {
        let release = p.release.clone().filter(|r| {
            r.version_code > identity.version_code
                && r.min_sdk <= identity.sdk
                && identity.abis.contains(&r.abi)
                && r.certificate_sha256
                    .eq_ignore_ascii_case(&identity.certificate_sha256)
                && https_url(&r.url).is_ok()
        });
        Status {
            enabled: true,
            automatic: p.automatic,
            feed_url: self.source(p),
            current_version: identity.version.clone(),
            current_version_code: identity.version_code,
            last_checked: p.last_checked,
            retry_at: p.retry_at,
            error: p.error.clone(),
            release,
            dismissed_version: p.dismissed_version,
        }
    }
    fn save(&self, preferences: &Preferences) -> Result<(), String> {
        use std::io::Write;
        let bytes = serde_json::to_vec(preferences).map_err(|e| e.to_string())?;
        let mut file =
            atomic_write_file::AtomicWriteFile::open(&self.path).map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.commit())
            .map_err(|e| e.to_string())
    }
    pub async fn get(&self, identity: &Identity) -> Status {
        self.status(&*self.preferences.lock().await, identity)
    }
    pub async fn configure(
        &self,
        identity: &Identity,
        automatic: bool,
        feed_url: String,
    ) -> Result<Status, String> {
        let source = if feed_url.trim().is_empty() {
            None
        } else {
            Some(https_url(&feed_url)?.to_string())
        };
        let mut p = self.preferences.lock().await;
        let mut next = p.clone();
        if self.source(&next) != source.clone().or_else(|| self.default_feed.clone()) {
            next.last_checked = None;
            next.retry_at = None;
            next.release = None;
            next.dismissed_version = None;
        }
        next.automatic = automatic;
        next.feed_url = source;
        next.error = None;
        self.save(&next)?;
        *p = next;
        Ok(self.status(&p, identity))
    }
    pub async fn dismiss(&self, identity: &Identity) -> Result<Status, String> {
        let mut p = self.preferences.lock().await;
        let mut next = p.clone();
        next.dismissed_version = next.release.as_ref().map(|r| r.version_code);
        self.save(&next)?;
        *p = next;
        Ok(self.status(&p, identity))
    }
    pub async fn check(&self, identity: &Identity, manual: bool) -> Result<Status, String> {
        let mut p = self.preferences.lock().await;
        let Some(source) = self.source(&p) else {
            return Ok(self.status(&p, identity));
        };
        let time = now();
        if p.retry_at.is_some_and(|until| until > time)
            || (!manual
                && (!p.automatic
                    || p.last_checked
                        .is_some_and(|last| time.saturating_sub(last) < INTERVAL)))
        {
            return Ok(self.status(&p, identity));
        }
        let source = https_url(&source)?;
        let mut next = p.clone();
        next.last_checked = Some(time);
        // Persist the attempt before networking; repeated restarts cannot spam a
        // failing server. Explicit checks still honor server cooldowns.
        self.save(&next)?;
        *p = next;
        match self.fetch(source).await {
            Ok((bytes, cooldown)) => {
                p.retry_at = cooldown;
                match bytes.and_then(|b| select(&b, identity)) {
                    Ok(release) => {
                        p.release = release;
                        p.error = None;
                        if manual {
                            p.dismissed_version = None;
                        }
                    }
                    Err(error) => p.error = Some(error),
                }
            }
            Err(error) => p.error = Some(format!("Could not check for updates: {error}")),
        }
        self.save(&p)?;
        Ok(self.status(&p, identity))
    }
    async fn fetch(
        &self,
        source: Url,
    ) -> Result<(Result<Vec<u8>, String>, Option<u64>), reqwest::Error> {
        let mut url = source.clone();
        for _ in 0..=3 {
            let mut response = self.client.get(url.clone()).send().await?;
            if response.status().is_redirection() {
                let target = response
                    .headers()
                    .get("location")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| url.join(s).ok());
                match target {
                    Some(target)
                        if https_url(target.as_str()).is_ok()
                            && target.origin() == source.origin()
                            && target.username().is_empty()
                            && target.password().is_none() =>
                    {
                        url = target;
                        continue;
                    }
                    _ => {
                        return Ok((
                            Err("The update feed redirected to an untrusted address".into()),
                            None,
                        ))
                    }
                }
            }
            if response.status() == StatusCode::TOO_MANY_REQUESTS
                || response.status().is_server_error()
            {
                let retry = response
                    .headers()
                    .get("retry-after")
                    .and_then(|h| h.to_str().ok());
                let delay = zim_engine::web::retry_delay(retry, 0).as_secs().max(60);
                return Ok((
                    Err(format!(
                        "Update server returned HTTP {}. Waiting before another check.",
                        response.status().as_u16()
                    )),
                    Some(now().saturating_add(delay).saturating_add(1)),
                ));
            }
            if !response.status().is_success() {
                return Ok((
                    Err(format!(
                        "Update server returned HTTP {}",
                        response.status().as_u16()
                    )),
                    None,
                ));
            }
            if response.content_length().is_some_and(|n| n > LIMIT as u64) {
                return Ok((Err("The release feed is too large".into()), None));
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if body.len() + chunk.len() > LIMIT {
                    return Ok((Err("The release feed is too large".into()), None));
                }
                body.extend_from_slice(&chunk);
            }
            return Ok((Ok(body), None));
        }
        Ok((Err("Too many update-feed redirects".into()), None))
    }
    pub async fn download_url(&self, identity: &Identity) -> Result<String, String> {
        let status = self.get(identity).await;
        let release = status
            .release
            .ok_or("Check for an available update first")?;
        Ok(https_url(&release.url)?.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity() -> Identity {
        Identity {
            package_id: "org.zimstudio.app".into(),
            version: "0.1.0".into(),
            version_code: 1000,
            sdk: 37,
            abis: vec!["arm64-v8a".into()],
            certificate_sha256: "a".repeat(64),
        }
    }
    fn release(code: u64) -> Release {
        Release {
            version: format!("0.1.{}", code - 1000),
            version_code: code,
            abi: "arm64-v8a".into(),
            min_sdk: 24,
            url: "https://downloads.example/app.apk".into(),
            sha256: "b".repeat(64),
            certificate_sha256: "a".repeat(64),
            size_bytes: 123,
            notes: "A release".into(),
        }
    }
    fn feed(releases: Vec<Release>) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"schema_version":1,"package_id":"org.zimstudio.app","channel":"direct","releases":releases})).unwrap()
    }
    #[test]
    fn release_selection_enforces_identity_compatibility_and_monotonic_versions() {
        let id = identity();
        assert!(select(&feed(vec![release(1000)]), &id).unwrap().is_none());
        let mut wrong_abi = release(1009);
        wrong_abi.abi = "x86_64".into();
        let mut new_sdk = release(1008);
        new_sdk.min_sdk = 99;
        assert_eq!(
            select(
                &feed(vec![release(1001), wrong_abi, new_sdk, release(1002)]),
                &id
            )
            .unwrap()
            .unwrap()
            .version_code,
            1002
        );
        let mut wrong_key = release(1001);
        wrong_key.certificate_sha256 = "c".repeat(64);
        assert!(select(&feed(vec![wrong_key]), &id)
            .unwrap_err()
            .contains("signing key"));
        let mut malicious = release(1001);
        malicious.url = "javascript:alert(1)".into();
        assert!(select(&feed(vec![malicious]), &id).is_err());
        let mut large = release(1001);
        large.notes = "x".repeat(8001);
        assert!(select(&feed(vec![large]), &id).is_err());
        let mut malformed = release(1001);
        malformed.sha256 = "bad".into();
        assert!(select(&feed(vec![malformed]), &id).is_err());
        let wrong = feed(vec![release(1001)]);
        let mut wrong: serde_json::Value = serde_json::from_slice(&wrong).unwrap();
        wrong["package_id"] = "other.app".into();
        assert!(select(&serde_json::to_vec(&wrong).unwrap(), &id).is_err());
        wrong["package_id"] = "org.zimstudio.app".into();
        wrong["channel"] = "store".into();
        assert!(select(&serde_json::to_vec(&wrong).unwrap(), &id).is_err());
    }
    #[test]
    fn feed_urls_cannot_launch_local_files_scripts_or_embed_credentials() {
        for url in [
            "http://host/feed",
            "file:///app.apk",
            "javascript:alert(1)",
            "https://user:secret@example.com/feed",
            "https://host/feed#fragment",
            "https://host/\nfeed",
        ] {
            assert!(https_url(url).is_err(), "{url}");
        }
        assert!(https_url("https://host.example/updates/android.json").is_ok());
    }
    #[tokio::test]
    async fn preferences_and_dismissals_survive_restart_and_source_changes_clear_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updates.json");
        let id = identity();
        let updater = Updater::new(path.clone(), Some("https://default.example/feed"));
        updater
            .configure(&id, false, "https://custom.example/feed".into())
            .await
            .unwrap();
        {
            let mut p = updater.preferences.lock().await;
            p.release = Some(release(1001));
            p.last_checked = Some(now());
            updater.save(&p).unwrap();
        }
        updater.dismiss(&id).await.unwrap();
        drop(updater);
        let updater = Updater::new(path, None);
        let status = updater.get(&id).await;
        assert!(!status.automatic);
        assert_eq!(status.dismissed_version, Some(1001));
        assert_eq!(
            status.feed_url.as_deref(),
            Some("https://custom.example/feed")
        );
        // Automatic and recent checks do not touch the network.
        updater.check(&id, false).await.unwrap();
        assert!(updater.get(&id).await.error.is_none());
        {
            let mut p = updater.preferences.lock().await;
            p.automatic = true;
            p.retry_at = Some(now() + 100);
        }
        updater.check(&id, true).await.unwrap();
        assert!(updater.get(&id).await.error.is_none());
        let changed = updater
            .configure(&id, true, "https://new.example/feed".into())
            .await
            .unwrap();
        assert!(changed.release.is_none());
        assert!(changed.last_checked.is_none());
        assert!(changed.retry_at.is_none());
        assert!(changed.dismissed_version.is_none());
        assert!(updater
            .configure(&id, true, "http://insecure.example/feed".into())
            .await
            .is_err());
        assert_eq!(updater.get(&id).await.feed_url, changed.feed_url);
    }
    #[tokio::test]
    async fn bounded_fetch_honors_retry_after_and_rejects_untrusted_redirects() {
        use axum::{http::StatusCode as AxumStatus, routing::get, Router};
        let router = Router::new()
            .route(
                "/rate",
                get(|| async {
                    (
                        AxumStatus::TOO_MANY_REQUESTS,
                        [("retry-after", "120")],
                        "slow down",
                    )
                }),
            )
            .route("/huge", get(|| async { "x".repeat(LIMIT + 1) }))
            .route(
                "/redirect",
                get(|| async {
                    (
                        AxumStatus::FOUND,
                        [("location", "https://other.example/feed")],
                        "",
                    )
                }),
            )
            .route("/ok", get(|| async { "{}" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        let updater = Updater::new(dir.path().join("updates.json"), None);
        // Only the private transport is exercised over loopback HTTP. Public
        // configuration and release validation always enforce HTTPS.
        let (body, retry) = updater
            .fetch(Url::parse(&format!("http://{addr}/rate")).unwrap())
            .await
            .unwrap();
        assert!(body.unwrap_err().contains("429"));
        assert!(retry.unwrap() >= now() + 119);
        let (body, _) = updater
            .fetch(Url::parse(&format!("http://{addr}/huge")).unwrap())
            .await
            .unwrap();
        assert!(body.unwrap_err().contains("too large"));
        let (body, _) = updater
            .fetch(Url::parse(&format!("http://{addr}/redirect")).unwrap())
            .await
            .unwrap();
        assert!(body.unwrap_err().contains("untrusted"));
        let (body, _) = updater
            .fetch(Url::parse(&format!("http://{addr}/ok")).unwrap())
            .await
            .unwrap();
        assert_eq!(body.unwrap(), b"{}");
        server.abort();
    }
}
