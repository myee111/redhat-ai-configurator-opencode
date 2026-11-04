//! Service endpoints, kept out of the source so a public build contains no
//! internal hostnames. Resolution order:
//!
//! 1. command-line overrides (`--rhai-maas-base-url`, `--enmaas-base-url`)
//! 2. environment variables
//! 3. `/etc/redhat-ai-configurator/endpoints.conf`
//! 4. the placeholder defaults below
//!
//! The defaults are deliberately examples, so a fresh install cannot silently
//! talk to the wrong service.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

pub const CONFIG_PATH: &str = "/etc/redhat-ai-configurator/endpoints.conf";

pub const KEYS: [&str; 8] = [
    "RHAI_MAAS_BASE_URL",
    "RHAI_MAAS_KEY_PAGE",
    "RHAI_MAAS_HELP_URL",
    "ENMAAS_BASE_URL",
    "ENMAAS_KEY_PAGE",
    "ENMAAS_HELP_URL",
    "ENMAAS_ELIGIBILITY_URL",
    "ENMAAS_SLACK",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub rhai_maas_base: String,
    pub rhai_maas_key_page: String,
    pub rhai_maas_help: String,
    pub enmaas_base: String,
    pub enmaas_key_page: String,
    pub enmaas_help: String,
    pub enmaas_eligibility: String,
    pub enmaas_slack: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            rhai_maas_base: "https://maas.example.com/v1".into(),
            rhai_maas_key_page: "https://maas.example.com/keys".into(),
            rhai_maas_help: "https://maas.example.com/docs".into(),
            enmaas_base: "https://enmaas.example.com".into(),
            enmaas_key_page: "https://enmaas.example.com/keys".into(),
            enmaas_help: "https://enmaas.example.com/docs".into(),
            enmaas_eligibility: String::new(),
            enmaas_slack: String::new(),
        }
    }
}

impl Endpoints {
    /// Build the effective endpoints, with command-line overrides winning over
    /// both the environment and the config file.
    pub fn resolve_with(
        file: &BTreeMap<String, String>,
        env: &BTreeMap<String, String>,
        overrides: Option<&BTreeMap<String, String>>,
    ) -> Self {
        let mut endpoints = Self::default();
        let value = |key: &str| {
            let clean = |map: &BTreeMap<String, String>| {
                map.get(key)
                    .map(|raw| raw.trim().to_string())
                    .filter(|raw| !raw.is_empty())
            };
            overrides
                .and_then(clean)
                .or_else(|| clean(env))
                .or_else(|| clean(file))
        };
        if let Some(v) = value("RHAI_MAAS_BASE_URL") {
            endpoints.rhai_maas_base = v;
        }
        if let Some(v) = value("RHAI_MAAS_KEY_PAGE") {
            endpoints.rhai_maas_key_page = v;
        }
        if let Some(v) = value("RHAI_MAAS_HELP_URL") {
            endpoints.rhai_maas_help = v;
        }
        if let Some(v) = value("ENMAAS_BASE_URL") {
            endpoints.enmaas_base = v;
        }
        if let Some(v) = value("ENMAAS_KEY_PAGE") {
            endpoints.enmaas_key_page = v;
        }
        if let Some(v) = value("ENMAAS_HELP_URL") {
            endpoints.enmaas_help = v;
        }
        if let Some(v) = value("ENMAAS_ELIGIBILITY_URL") {
            endpoints.enmaas_eligibility = v;
        }
        if let Some(v) = value("ENMAAS_SLACK") {
            endpoints.enmaas_slack = v;
        }
        endpoints
    }

    pub fn provider_base(&self, provider: &str) -> &str {
        match provider {
            "rhai-maas" => &self.rhai_maas_base,
            _ => &self.enmaas_base,
        }
    }

    pub fn key_page(&self, provider: &str) -> &str {
        match provider {
            "rhai-maas" => &self.rhai_maas_key_page,
            _ => &self.enmaas_key_page,
        }
    }

    pub fn help(&self, provider: &str) -> &str {
        match provider {
            "rhai-maas" => &self.rhai_maas_help,
            _ => &self.enmaas_help,
        }
    }

    pub fn eligibility(&self, provider: &str) -> Option<&str> {
        (provider == "enmaas" && !self.enmaas_eligibility.is_empty())
            .then_some(self.enmaas_eligibility.as_str())
    }

    pub fn slack(&self, provider: &str) -> Option<&str> {
        (provider == "enmaas" && !self.enmaas_slack.is_empty())
            .then_some(self.enmaas_slack.as_str())
    }

    /// True when any of the given providers still points at a placeholder.
    pub fn uses_placeholders_for<'a>(
        &self,
        providers: impl IntoIterator<Item = &'a str>,
    ) -> bool {
        providers
            .into_iter()
            .any(|provider| self.provider_base(provider).contains("example.com"))
    }

    /// Warn when a base URL does not have the shape goose expects. This is a
    /// warning rather than an error because the value may come from a shared
    /// config file that this run must not break.
    pub fn shape_warning(&self, provider: &str) -> Option<String> {
        let base = self.provider_base(provider).trim_end_matches('/');
        if base.contains("example.com") {
            return None;
        }
        if provider == "rhai-maas" && !base.ends_with("/v1") {
            return Some(format!(
                "RHAI_MAAS_BASE_URL should include /v1 (currently {base}); goose appends \
                 /chat/completions to it."
            ));
        }
        if provider != "rhai-maas" && base.ends_with("/v1") {
            return Some(format!(
                "ENMAAS_BASE_URL should be the host root without /v1 (currently {base}); goose \
                 appends v1/messages, which would become v1/v1/messages."
            ));
        }
        None
    }
}

fn parse_config(text: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            if KEYS.contains(&key) {
                values.insert(key.to_string(), value.trim().to_string());
            }
        }
    }
    values
}

fn read_config(path: &Path) -> BTreeMap<String, String> {
    fs::read_to_string(path)
        .map(|text| parse_config(&text))
        .unwrap_or_default()
}

fn env_values() -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for key in KEYS {
        if let Ok(value) = std::env::var(key) {
            values.insert(key.to_string(), value);
        }
    }
    values
}

static CACHE: OnceLock<Endpoints> = OnceLock::new();

/// Base URLs supplied on the command line. Installed once, before the first
/// `endpoints()` call.
static CLI_OVERRIDES: OnceLock<BTreeMap<String, String>> = OnceLock::new();

static NO_OVERRIDES: BTreeMap<String, String> = BTreeMap::new();

/// Install endpoint overrides from the command line. Later calls are ignored,
/// because the effective endpoints are resolved only once per process.
pub fn set_overrides(overrides: BTreeMap<String, String>) {
    let _ = CLI_OVERRIDES.set(overrides);
}

pub fn endpoints() -> &'static Endpoints {
    CACHE.get_or_init(|| {
        let overrides = CLI_OVERRIDES.get().unwrap_or(&NO_OVERRIDES);
        Endpoints::resolve_with(
            &read_config(Path::new(CONFIG_PATH)),
            &env_values(),
            Some(overrides),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_are_placeholders() {
        let endpoints = Endpoints::resolve_with(&BTreeMap::new(), &BTreeMap::new(), None);
        assert!(endpoints.uses_placeholders_for(["rhai-maas", "enmaas"]));
        assert_eq!(endpoints.provider_base("rhai-maas"), "https://maas.example.com/v1");
        assert_eq!(endpoints.provider_base("enmaas"), "https://enmaas.example.com");
        assert_eq!(endpoints.eligibility("enmaas"), None);
    }

    #[test]
    fn placeholder_check_is_per_provider() {
        let env = map(&[("RHAI_MAAS_BASE_URL", "https://maas.corp.test/v1")]);
        let endpoints = Endpoints::resolve_with(&BTreeMap::new(), &env, None);
        assert!(!endpoints.uses_placeholders_for(["rhai-maas"]));
        assert!(endpoints.uses_placeholders_for(["enmaas"]));
        assert!(endpoints.uses_placeholders_for(["rhai-maas", "enmaas"]));
    }

    #[test]
    fn environment_overrides_the_file() {
        let file = map(&[("RHAI_MAAS_BASE_URL", "https://file.example/v1")]);
        let env = map(&[
            ("RHAI_MAAS_BASE_URL", "https://env.example/v1"),
            ("ENMAAS_SLACK", "#help"),
        ]);
        let endpoints = Endpoints::resolve_with(&file, &env, None);
        assert_eq!(endpoints.rhai_maas_base, "https://env.example/v1");
        assert_eq!(endpoints.slack("enmaas"), Some("#help"));
    }

    #[test]
    fn blank_values_are_ignored() {
        let env = map(&[("ENMAAS_BASE_URL", "   ")]);
        let endpoints = Endpoints::resolve_with(&BTreeMap::new(), &env, None);
        assert_eq!(endpoints.enmaas_base, "https://enmaas.example.com");
    }

    #[test]
    fn command_line_overrides_win_over_the_environment() {
        let file = map(&[("RHAI_MAAS_BASE_URL", "https://file.example/v1")]);
        let env = map(&[("RHAI_MAAS_BASE_URL", "https://env.example/v1")]);
        let overrides = map(&[("RHAI_MAAS_BASE_URL", "https://cli.example/v1")]);
        let endpoints = Endpoints::resolve_with(&file, &env, Some(&overrides));
        assert_eq!(endpoints.rhai_maas_base, "https://cli.example/v1");
        assert_eq!(endpoints.enmaas_base, "https://enmaas.example.com");
    }

    #[test]
    fn blank_overrides_do_not_mask_a_configured_value() {
        let env = map(&[("ENMAAS_BASE_URL", "https://enmaas.corp.test")]);
        let overrides = map(&[("ENMAAS_BASE_URL", "  ")]);
        let endpoints = Endpoints::resolve_with(&BTreeMap::new(), &env, Some(&overrides));
        assert_eq!(endpoints.enmaas_base, "https://enmaas.corp.test");
    }

    #[test]
    fn shape_warning_flags_the_wrong_suffix_for_each_provider() {
        let env = map(&[
            ("RHAI_MAAS_BASE_URL", "https://maas.corp.test"),
            ("ENMAAS_BASE_URL", "https://enmaas.corp.test/v1"),
        ]);
        let endpoints = Endpoints::resolve_with(&BTreeMap::new(), &env, None);
        assert!(endpoints.shape_warning("rhai-maas").is_some());
        assert!(endpoints.shape_warning("enmaas").is_some());
    }

    #[test]
    fn shape_warning_is_quiet_for_correct_and_placeholder_bases() {
        let fresh = Endpoints::resolve_with(&BTreeMap::new(), &BTreeMap::new(), None);
        assert_eq!(fresh.shape_warning("rhai-maas"), None);
        assert_eq!(fresh.shape_warning("enmaas"), None);

        let env = map(&[
            ("RHAI_MAAS_BASE_URL", "https://maas.corp.test/v1/"),
            ("ENMAAS_BASE_URL", "https://enmaas.corp.test/"),
        ]);
        let endpoints = Endpoints::resolve_with(&BTreeMap::new(), &env, None);
        assert_eq!(endpoints.shape_warning("rhai-maas"), None);
        assert_eq!(endpoints.shape_warning("enmaas"), None);
    }

    #[test]
    fn config_parser_ignores_comments_and_unknown_keys() {
        let values = parse_config(
            "# comment\nRHAI_MAAS_BASE_URL=https://example.test/v1\nOTHER=1\nbad line\n",
        );
        assert_eq!(values.len(), 1);
        assert_eq!(
            values.get("RHAI_MAAS_BASE_URL").map(String::as_str),
            Some("https://example.test/v1")
        );
    }

    #[test]
    fn placeholders_clear_once_a_real_base_is_configured() {
        let env = map(&[
            ("RHAI_MAAS_BASE_URL", "https://maas.corp.test/v1"),
            ("ENMAAS_BASE_URL", "https://enmaas.corp.test"),
        ]);
        assert!(
            !Endpoints::resolve_with(&BTreeMap::new(), &env, None)
                .uses_placeholders_for(["rhai-maas", "enmaas"])
        );
    }
}
