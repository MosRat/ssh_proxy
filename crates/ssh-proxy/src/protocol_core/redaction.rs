use serde_json::Value;

pub(crate) fn redact_value(value: &Value) -> Value {
    crate::peer_lifecycle::report::redact_value(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redaction_reuses_lifecycle_rules() {
        let value = json!({
            "auth_token": "secret",
            "identity_file": "C:/Users/me/.ssh/id_rsa",
            "safe": "ok"
        });
        let redacted = redact_value(&value);

        assert_eq!(redacted["auth_token"], "<redacted>");
        assert_eq!(redacted["identity_file"], "<redacted>/id_rsa");
        assert_eq!(redacted["safe"], "ok");
    }
}
