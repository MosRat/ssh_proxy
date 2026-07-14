use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalActionClass {
    RequiredProvider,
    FallbackProvider,
    DiagnosticOnly,
    SelfUpdate,
    EmergencyCompat,
}

impl ExternalActionClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RequiredProvider => "required_provider",
            Self::FallbackProvider => "fallback_provider",
            Self::DiagnosticOnly => "diagnostic_only",
            Self::SelfUpdate => "self_update",
            Self::EmergencyCompat => "emergency_compat",
        }
    }
}

impl fmt::Display for ExternalActionClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ExternalActionClass {
    type Err = ParseExternalActionClassError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "required_provider" => Ok(Self::RequiredProvider),
            "fallback_provider" => Ok(Self::FallbackProvider),
            "diagnostic_only" => Ok(Self::DiagnosticOnly),
            "self_update" => Ok(Self::SelfUpdate),
            "emergency_compat" => Ok(Self::EmergencyCompat),
            _ => Err(ParseExternalActionClassError {
                value: value.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseExternalActionClassError {
    value: String,
}

impl fmt::Display for ParseExternalActionClassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown external action class `{}`", self.value)
    }
}

impl std::error::Error for ParseExternalActionClassError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalActionReport {
    pub class: ExternalActionClass,
    pub execution_backend: String,
    pub fallback_used: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_action: Option<String>,
}

impl ExternalActionReport {
    pub fn new(
        class: ExternalActionClass,
        execution_backend: impl Into<String>,
        fallback_used: bool,
    ) -> Self {
        Self {
            class,
            execution_backend: execution_backend.into(),
            fallback_used,
            reason: None,
            repair_action: None,
        }
    }

    pub fn required_provider(execution_backend: impl Into<String>) -> Self {
        Self::new(
            ExternalActionClass::RequiredProvider,
            execution_backend,
            false,
        )
    }

    pub fn fallback_provider(execution_backend: impl Into<String>) -> Self {
        Self::new(
            ExternalActionClass::FallbackProvider,
            execution_backend,
            true,
        )
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_repair_action(mut self, repair_action: impl Into<String>) -> Self {
        self.repair_action = Some(repair_action.into());
        self
    }

    pub fn with_optional_repair_action(mut self, repair_action: Option<String>) -> Self {
        self.repair_action = repair_action;
        self
    }

    pub fn to_json(&self) -> Value {
        json!(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_action_class_round_trips_snake_case() {
        for class in [
            ExternalActionClass::RequiredProvider,
            ExternalActionClass::FallbackProvider,
            ExternalActionClass::DiagnosticOnly,
            ExternalActionClass::SelfUpdate,
            ExternalActionClass::EmergencyCompat,
        ] {
            assert_eq!(
                class.as_str().parse::<ExternalActionClass>().unwrap(),
                class
            );
        }
    }

    #[test]
    fn external_action_report_serializes_operability_fields() {
        let report = ExternalActionReport::fallback_provider("provider_command")
            .with_reason("systemctl fallback after dbus unavailable")
            .with_repair_action("install systemd dbus support");
        let value = report.to_json();

        assert_eq!(value["class"], "fallback_provider");
        assert_eq!(value["execution_backend"], "provider_command");
        assert_eq!(value["fallback_used"], true);
        assert_eq!(value["reason"], "systemctl fallback after dbus unavailable");
        assert_eq!(value["repair_action"], "install systemd dbus support");
    }
}
