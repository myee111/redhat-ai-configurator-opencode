//! The provider catalog: what the installer knows about the two MaaS
//! services. Endpoints live in `settings` so this file contains no hostnames.

use crate::settings;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Engine {
    OpenAi,
    Anthropic,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Engine::OpenAi => "openai",
            Engine::Anthropic => "anthropic",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VpnNeed {
    Off,
    On,
}

impl VpnNeed {
    pub fn text(self) -> &'static str {
        match self {
            VpnNeed::Off => "VPN off",
            VpnNeed::On => "VPN on",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Model {
    /// Model name exactly as the service expects it.
    pub name: &'static str,
    pub display: &'static str,
    /// Declarative provider that serves this model.
    pub provider: &'static str,
    /// Required by goose for every model entry.
    pub context_limit: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Declarative {
    pub name: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub engine: Engine,
    /// Appended to the provider's configured base URL.
    pub base_suffix: &'static str,
    pub api_key_env: &'static str,
    pub models: &'static [Model],
}

impl Declarative {
    pub fn base_url(&self) -> String {
        format!(
            "{}{}",
            settings::endpoints().provider_base(self.name).trim_end_matches('/'),
            self.base_suffix
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProviderSpec {
    pub id: &'static str,
    pub display: &'static str,
    /// Internal handle shown in the UI.
    pub handle: &'static str,
    pub vpn: VpnNeed,
    /// Name of the secret goose looks up for this provider.
    pub secret: &'static str,
    pub declaratives: &'static [Declarative],
    pub default_provider: &'static str,
    pub default_model: &'static str,
}

impl ProviderSpec {
    /// Base URL of the provider's primary declarative entry.
    pub fn base_url(&self) -> String {
        self.declaratives
            .first()
            .map(Declarative::base_url)
            .unwrap_or_default()
    }

    /// `GET` endpoint used to validate a key.
    pub fn validate_url(&self) -> String {
        let base = self.base_url();
        let base = base.trim_end_matches('/');
        if self.id == "rhai-maas" {
            // The OpenAI base already ends in /v1.
            format!("{base}/models")
        } else {
            // The Anthropic base is the host root; goose appends v1/messages.
            format!("{base}/v1/models")
        }
    }

    pub fn key_page(&self) -> String {
        settings::endpoints().key_page(self.id).to_string()
    }

    pub fn help_url(&self) -> String {
        settings::endpoints().help(self.id).to_string()
    }

    pub fn eligibility_url(&self) -> Option<String> {
        settings::endpoints().eligibility(self.id).map(str::to_string)
    }

    pub fn slack(&self) -> Option<String> {
        settings::endpoints().slack(self.id).map(str::to_string)
    }
}

const RHAI_MAAS_MODELS: &[Model] = &[
    Model {
        name: "publishers/prelude-maas/models/glm-53-flash",
        display: "GLM 5.3 Flash",
        provider: "rhai-maas",
        context_limit: 200_000,
    },
    Model {
        name: "publishers/prelude-maas/models/qwen38-27b",
        display: "Qwen 3.8 27B",
        provider: "rhai-maas",
        context_limit: 200_000,
    },
    Model {
        name: "publishers/prelude-maas/models/llama-4-scout-17b-16e-w4a16",
        display: "Llama 4 Scout 17B",
        provider: "rhai-maas",
        context_limit: 200_000,
    },
    Model {
        name: "publishers/prelude-maas/models/nemotron-cascade-2-30b",
        display: "Nemotron Cascade 2 30B",
        provider: "rhai-maas",
        context_limit: 200_000,
    },
    Model {
        name: "publishers/prelude-maas/models/gemma4",
        display: "Gemma 4",
        provider: "rhai-maas",
        context_limit: 200_000,
    },
    Model {
        name: "publishers/prelude-maas/models/llama-32-3b",
        display: "Llama 3.2 3B",
        provider: "rhai-maas",
        context_limit: 200_000,
    },
];

const ENMAAS_MODELS: &[Model] = &[
    Model {
        name: "rits/zai-org/glm-5-3",
        display: "GLM 5.3",
        provider: "enmaas",
        context_limit: 1_000_000,
    },
    Model {
        name: "rits/deepseek-ai/DeepSeek-V4.1-Flash-1017",
        display: "DeepSeek 4.1 Flash",
        provider: "enmaas",
        context_limit: 1_000_000,
    },
    Model {
        name: "claude-opus-5-5",
        display: "Claude Opus 5.5",
        provider: "enmaas",
        context_limit: 200_000,
    },
    Model {
        name: "claude-opus-5",
        display: "Claude Opus 5",
        provider: "enmaas",
        context_limit: 200_000,
    },
    Model {
        name: "claude-opus-4-8",
        display: "Claude Opus 4.8",
        provider: "enmaas",
        context_limit: 200_000,
    },
    Model {
        name: "claude-opus-4-6",
        display: "Claude Opus 4.6",
        provider: "enmaas",
        context_limit: 200_000,
    },
    Model {
        name: "claude-sonnet-5-5",
        display: "Claude Sonnet 5.5",
        provider: "enmaas",
        context_limit: 200_000,
    },
    Model {
        name: "claude-sonnet-5",
        display: "Claude Sonnet 5",
        provider: "enmaas",
        context_limit: 200_000,
    },
    Model {
        name: "claude-sonnet-4-6",
        display: "Claude Sonnet 4.6",
        provider: "enmaas",
        context_limit: 200_000,
    },
    Model {
        name: "claude-haiku-4-5",
        display: "Claude Haiku 4.5",
        provider: "enmaas",
        context_limit: 200_000,
    },
];

const ENMAAS_OPENAI_MODELS: &[Model] = &[
    Model {
        name: "gemini-3.1-pro-preview",
        display: "Gemini 3.1 Pro",
        provider: "enmaas-openai",
        context_limit: 1_000_000,
    },
    Model {
        name: "gemini-3.8-flash",
        display: "Gemini 3.8 Flash",
        provider: "enmaas-openai",
        context_limit: 1_000_000,
    },
    Model {
        name: "gemini-3.7-flash",
        display: "Gemini 3.7 Flash",
        provider: "enmaas-openai",
        context_limit: 1_000_000,
    },
    Model {
        name: "gemini-3.6-flash",
        display: "Gemini 3.6 Flash",
        provider: "enmaas-openai",
        context_limit: 1_000_000,
    },
];

const RHAI_MAAS_DECLARATIVES: &[Declarative] = &[Declarative {
    name: "rhai-maas",
    display_name: "Red Hat AI MaaS (rhai-maas)",
    description: "Red Hat AI Model-as-a-Service, OpenAI compatible",
    engine: Engine::OpenAi,
    base_suffix: "",
    api_key_env: "RHAI_MAAS_API_KEY",
    models: RHAI_MAAS_MODELS,
}];

const ENMAAS_DECLARATIVES: &[Declarative] = &[
    Declarative {
        name: "enmaas",
        display_name: "EnMaaS",
        description: "EnMaaS Anthropic-compatible endpoint",
        engine: Engine::Anthropic,
        // goose joins "v1/messages" to this root, so no /v1 here.
        base_suffix: "",
        api_key_env: "ENMAAS_API_KEY",
        models: ENMAAS_MODELS,
    },
    Declarative {
        name: "enmaas-openai",
        display_name: "EnMaaS (OpenAI compatible)",
        description: "EnMaaS OpenAI-compatible endpoint for Gemini models",
        engine: Engine::OpenAi,
        base_suffix: "/v1",
        api_key_env: "ENMAAS_API_KEY",
        models: ENMAAS_OPENAI_MODELS,
    },
];

pub const PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        id: "rhai-maas",
        display: "Red Hat AI MaaS",
        handle: "rhai-maas",
        vpn: VpnNeed::Off,
        secret: "RHAI_MAAS_API_KEY",
        declaratives: RHAI_MAAS_DECLARATIVES,
        default_provider: "rhai-maas",
        default_model: "publishers/prelude-maas/models/glm-53-flash",
    },
    ProviderSpec {
        id: "enmaas",
        display: "EnMaaS",
        handle: "enmaas",
        vpn: VpnNeed::On,
        secret: "ENMAAS_API_KEY",
        declaratives: ENMAAS_DECLARATIVES,
        default_provider: "enmaas",
        default_model: "rits/zai-org/glm-5-3",
    },
];

pub fn all() -> &'static [ProviderSpec] {
    PROVIDERS
}

pub fn find(id: &str) -> Option<&'static ProviderSpec> {
    let id = normalize_id(id)?;
    PROVIDERS.iter().find(|p| p.id == id)
}

/// Accept the provider id and the friendly aliases people type.
pub fn normalize_id(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "rhai-maas" | "maas" | "ai-bu" | "rhai" => Some("rhai-maas"),
        "enmaas" | "enmaas-anthropic" => Some("enmaas"),
        _ => None,
    }
}

/// Find the declarative provider and model entry for a chosen model name.
pub fn resolve_model(
    spec: &ProviderSpec,
    model_name: &str,
) -> Option<(&'static Declarative, &'static Model)> {
    for d in spec.declaratives {
        if let Some(m) = d.models.iter().find(|m| m.name == model_name) {
            return Some((d, m));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_resolve() {
        assert_eq!(find("maas").map(|p| p.id), Some("rhai-maas"));
        assert_eq!(find("rhai-maas").map(|p| p.id), Some("rhai-maas"));
        assert_eq!(find("enmaas").map(|p| p.id), Some("enmaas"));
        assert!(find("nope").is_none());
    }

    #[test]
    fn every_default_model_exists() {
        for spec in all() {
            let (decl, model) = resolve_model(spec, spec.default_model)
                .unwrap_or_else(|| panic!("{} default model missing", spec.id));
            assert_eq!(decl.name, spec.default_provider);
            assert_eq!(model.name, spec.default_model);
        }
    }

    #[test]
    fn enmaas_shares_one_secret() {
        for d in find("enmaas").unwrap().declaratives {
            assert_eq!(d.api_key_env, "ENMAAS_API_KEY");
        }
    }

    #[test]
    fn endpoint_shapes_are_correct() {
        let rhai = find("rhai-maas").unwrap();
        let enmaas = find("enmaas").unwrap();
        assert!(rhai.base_url().ends_with("/v1"));
        assert!(rhai.validate_url().ends_with("/v1/models"));
        assert!(!enmaas.base_url().trim_end_matches('/').ends_with("/v1"));
        assert!(enmaas.validate_url().ends_with("/v1/models"));
        assert_eq!(
            enmaas.declaratives[1].base_url(),
            format!("{}/v1", enmaas.base_url().trim_end_matches('/'))
        );
        assert!(!rhai.key_page().is_empty());
        assert!(!rhai.help_url().is_empty());
    }
}
