use serde_json::{Map, Value, json};

pub fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(redact_object(object)),
        Value::Array(array) => Value::Array(array.iter().map(redact_value).collect()),
        other => other.clone(),
    }
}

fn redact_object(object: &Map<String, Value>) -> Map<String, Value> {
    let mut redacted = Map::new();
    for (key, value) in object {
        let lower = key.to_ascii_lowercase();
        if lower.contains("token")
            || lower.contains("password")
            || lower.contains("passphrase")
            || lower.contains("secret")
            || lower.contains("credential")
        {
            redacted.insert(key.clone(), json!("<redacted>"));
            continue;
        }
        if lower.contains("identity") || lower.contains("known_hosts") {
            redacted.insert(key.clone(), redact_pathish(value));
            continue;
        }
        redacted.insert(key.clone(), redact_value(value));
    }
    redacted
}

fn redact_pathish(value: &Value) -> Value {
    match value {
        Value::String(path) => json!(redacted_path(path)),
        Value::Array(values) => Value::Array(values.iter().map(redact_pathish).collect()),
        _ => redact_value(value),
    }
}

fn redacted_path(path: &str) -> String {
    let file = std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<path>");
    format!("<redacted>/{file}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redaction_hides_tokens_and_keeps_path_basenames() {
        let value = json!({
            "token": "secret",
            "identity": "C:/Users/me/.ssh/id_ed25519",
            "nested": {
                "known_hosts": ["C:/Users/me/.ssh/known_hosts"],
                "password": "also-secret",
                "safe": "ok"
            }
        });

        let redacted = redact_value(&value);

        assert_eq!(redacted["token"], "<redacted>");
        assert_eq!(redacted["identity"], "<redacted>/id_ed25519");
        assert_eq!(
            redacted["nested"]["known_hosts"][0],
            "<redacted>/known_hosts"
        );
        assert_eq!(redacted["nested"]["password"], "<redacted>");
        assert_eq!(redacted["nested"]["safe"], "ok");
    }
}
