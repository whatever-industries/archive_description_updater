//! Minimal Internet Archive client: log in, list your items, patch metadata.

use reqwest::blocking::Client;
use serde_json::Value;
use std::time::Duration;

const HOST: &str = "https://archive.org";
// Derived from the package so it cannot drift if the app is renamed again.
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

pub type Result<T> = std::result::Result<T, String>;

/// IA-S3 credentials. These are what actually authorise metadata writes; the
/// password is exchanged for them once at login and never stored.
#[derive(Clone, Debug)]
pub struct Creds {
    pub access: String,
    pub secret: String,
    pub screenname: String,
    pub email: String,
}

/// One item belonging to the logged-in user.
#[derive(Clone, Debug)]
pub struct Item {
    pub identifier: String,
    pub title: String,
    pub description: String,
}

pub fn client() -> Client {
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(60))
        .build()
        .expect("failed to build HTTP client")
}

/// Exchange an archive.org email + password for IA-S3 keys.
pub fn login(client: &Client, email: &str, password: &str) -> Result<Creds> {
    let resp = client
        .post(format!("{HOST}/services/xauthn/"))
        .query(&[("op", "login")])
        .form(&[("email", email), ("password", password)])
        .send()
        .map_err(|e| format!("network error: {e}"))?;

    let json: Value = resp
        .json()
        .map_err(|e| format!("unexpected response from archive.org: {e}"))?;

    if json.get("success").and_then(Value::as_bool) != Some(true) {
        // IA reports the cause under values.reason, falling back to error.
        let reason = json
            .pointer("/values/reason")
            .and_then(Value::as_str)
            .or_else(|| json.get("error").and_then(Value::as_str))
            .unwrap_or("unknown reason");
        return Err(match reason {
            "account_not_found" => "Account not found - check your email.".into(),
            "account_bad_password" => "Incorrect password - try again.".into(),
            other => format!("Login failed: {other}"),
        });
    }

    let get = |p: &str| -> Result<String> {
        json.pointer(p)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("login response missing {p}"))
    };

    Ok(Creds {
        access: get("/values/s3/access")?,
        secret: get("/values/s3/secret")?,
        screenname: json
            .pointer("/values/screenname")
            .and_then(Value::as_str)
            .unwrap_or(email)
            .to_owned(),
        email: email.to_owned(),
    })
}

/// Verify a pair of S3 keys pasted directly from archive.org/account/s3.php.
pub fn login_with_keys(client: &Client, access: &str, secret: &str) -> Result<Creds> {
    let resp = client
        .get(format!("{HOST}/account/info"))
        .header("Authorization", format!("LOW {access}:{secret}"))
        .query(&[("output", "json")])
        .send()
        .map_err(|e| format!("network error: {e}"))?;

    if !resp.status().is_success() {
        return Err("Those S3 keys were rejected by archive.org.".into());
    }
    let json: Value = resp
        .json()
        .map_err(|e| format!("unexpected response from archive.org: {e}"))?;

    let email = json
        .pointer("/values/email")
        .or_else(|| json.get("email"))
        .and_then(Value::as_str)
        .ok_or("Could not read the account for those keys.")?
        .to_owned();
    let screenname = json
        .pointer("/values/screenname")
        .or_else(|| json.get("screenname"))
        .and_then(Value::as_str)
        .unwrap_or(&email)
        .to_owned();

    Ok(Creds {
        access: access.to_owned(),
        secret: secret.to_owned(),
        screenname,
        email,
    })
}

/// Every item uploaded by this account, with descriptions, via the scrape API.
/// `on_page` reports the running total so the UI can show progress.
pub fn list_items(
    client: &Client,
    creds: &Creds,
    mut on_page: impl FnMut(usize, usize),
) -> Result<Vec<Item>> {
    let query = format!("uploader:\"{}\"", creds.email);
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let mut req = client
            .get(format!("{HOST}/services/search/v1/scrape"))
            .header(
                "Authorization",
                format!("LOW {}:{}", creds.access, creds.secret),
            )
            .query(&[
                ("q", query.as_str()),
                ("fields", "identifier,title,description"),
                ("count", "1000"),
            ]);
        if let Some(c) = &cursor {
            req = req.query(&[("cursor", c.as_str())]);
        }

        let resp = req.send().map_err(|e| format!("search failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("search failed: HTTP {}", resp.status()));
        }
        let json: Value = resp
            .json()
            .map_err(|e| format!("could not parse search results: {e}"))?;

        let total = json.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
        for it in json
            .get("items")
            .and_then(Value::as_array)
            .unwrap_or(&Vec::new())
        {
            items.push(Item {
                identifier: it
                    .get("identifier")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                title: flatten(it.get("title")),
                description: flatten(it.get("description")),
            });
        }
        on_page(items.len(), total.max(items.len()));

        match json.get("cursor").and_then(Value::as_str) {
            Some(c) => cursor = Some(c.to_owned()),
            None => break,
        }
    }

    Ok(items)
}

/// IA fields may be a string or a list of strings; render either as one string.
fn flatten(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The item's authoritative metadata, read fresh before any write.
pub fn get_metadata(client: &Client, identifier: &str) -> Result<Value> {
    let resp = client
        .get(format!("{HOST}/metadata/{identifier}"))
        .send()
        .map_err(|e| format!("could not read metadata: {e}"))?;
    let json: Value = resp
        .json()
        .map_err(|e| format!("could not parse metadata: {e}"))?;
    if json.get("metadata").is_none() {
        return Err("item is dark or does not exist".into());
    }
    Ok(json)
}

/// Replace the item's `description` via the Metadata Write API.
pub fn patch_description(
    client: &Client,
    creds: &Creds,
    identifier: &str,
    new_description: &Value,
) -> Result<()> {
    let patch = serde_json::json!([{
        "op": "replace",
        "path": "/description",
        "value": new_description,
    }]);

    let resp = client
        .post(format!("{HOST}/metadata/{identifier}"))
        .form(&[
            ("-target", "metadata"),
            ("-patch", &patch.to_string()),
            ("access", &creds.access),
            ("secret", &creds.secret),
        ])
        .send()
        .map_err(|e| format!("write failed: {e}"))?;

    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    let json: Value = serde_json::from_str(&body).unwrap_or(Value::Null);

    if json.get("success").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let err = json
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if body.is_empty() {
                format!("HTTP {status}")
            } else {
                body.chars().take(200).collect()
            }
        });
    Err(err)
}
