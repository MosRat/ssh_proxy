use anyhow::Result;

pub(super) fn pretty_json_line(value: serde_json::Value) -> Result<String> {
    Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
}
