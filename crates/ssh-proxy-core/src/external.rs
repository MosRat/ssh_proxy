use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

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
}
