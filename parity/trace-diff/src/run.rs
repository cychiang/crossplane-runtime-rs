use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Write {
    pub verb: String,
    #[serde(default)]
    pub api_group: Option<String>,
    pub resource: String,
    #[serde(default)]
    pub subresource: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub code: u16,
    #[serde(default)]
    pub request_object: Option<Value>,
    pub request_received_timestamp: String,
}

impl Write {
    pub fn succeeded(&self) -> bool {
        (200..300).contains(&self.code)
    }

    pub fn is_event(&self) -> bool {
        self.resource == "events" && matches!(self.api_group.as_deref(), None | Some("") | Some("events.k8s.io"))
    }

    /// Fixed-width seconds plus nanoseconds, because RFC3339Nano trims
    /// trailing zeros and so does not sort as a string.
    pub fn order_key(&self) -> (String, u32) {
        let ts = self.request_received_timestamp.trim_end_matches('Z');
        let (secs, frac) = ts.split_once('.').unwrap_or((ts, ""));
        let nanos = format!("{frac:0<9}").get(..9).and_then(|f| f.parse().ok()).unwrap_or(0);
        (secs.to_string(), nanos)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Call {
    pub seq: u64,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub body: Option<Value>,
    pub status: u16,
    #[serde(default)]
    pub fault: bool,
    pub user_agent: Option<String>,
    #[serde(default)]
    pub forwarded_for: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Report {
    #[serde(default)]
    pub tests: Vec<Test>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Test {
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    pub name: String,
    pub status: String,
}

pub struct Run {
    pub writes: Vec<Write>,
    pub calls: Vec<Call>,
    pub report: Report,
}

fn jsonl<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("{}:{}: {e}", path.display(), i + 1)))
        .collect()
}

pub fn load(dir: &Path, with_calls: bool) -> Result<Run, String> {
    let writes = jsonl(&dir.join("audit.jsonl"))?;
    let calls: Vec<Call> = if with_calls {
        jsonl(&dir.join("calls.jsonl"))?
    } else {
        vec![]
    };
    if let Some(c) = calls.iter().find(|c| c.user_agent.is_none()) {
        return Err(format!(
            "{}: call {} has no userAgent field, so the scenarios' own calls cannot be told apart; re-record with the current fake-external",
            dir.join("calls.jsonl").display(),
            c.seq
        ));
    }
    let report_path = dir.join("chainsaw-report.json");
    let text = fs::read_to_string(&report_path).map_err(|e| format!("{}: {e}", report_path.display()))?;
    let report = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", report_path.display()))?;
    Ok(Run { writes, calls, report })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ts: &str) -> Write {
        serde_json::from_value(serde_json::json!({
            "verb": "update", "resource": "r", "code": 200, "requestReceivedTimestamp": ts
        }))
        .unwrap()
    }

    #[test]
    fn a_call_with_an_empty_user_agent_is_current_format() {
        let c: Call =
            serde_json::from_str(r#"{"seq":1,"method":"GET","path":"/things/a","status":404,"userAgent":""}"#).unwrap();
        assert_eq!(c.user_agent.as_deref(), Some(""));
    }

    #[test]
    fn order_key_sorts_trimmed_fractions_numerically() {
        assert!(at("2026-11-09T22:43:19.1Z").order_key() < at("2026-11-09T22:43:19.12Z").order_key());
        assert!(at("2026-11-09T22:43:19Z").order_key() < at("2026-11-09T22:43:19.000001Z").order_key());
    }
}
