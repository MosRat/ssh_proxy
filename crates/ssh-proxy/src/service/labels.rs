use crate::cli;

use super::plan;

pub(super) fn service_scope_name(scope: plan::ServiceScope) -> &'static str {
    match scope {
        plan::ServiceScope::User => "user",
        plan::ServiceScope::System => "system",
    }
}

pub(super) fn cli_service_scope_name(scope: cli::ServiceScope) -> &'static str {
    match scope {
        cli::ServiceScope::Auto => "auto",
        cli::ServiceScope::User => "user",
        cli::ServiceScope::System => "system",
    }
}

pub(super) fn platform_service_name(scope: plan::ServiceScope) -> String {
    plan::platform_service_name(scope)
}
