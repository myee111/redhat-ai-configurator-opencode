//! The provider catalog: what the installer knows about the two MaaS
//! services and how OpenCode should address them. Endpoints live in `settings`
//! so this file contains no hostnames.

use crate::settings;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Engine {
    OpenAi,
    Anthropic,
}

impl Engine {
    /// Runtime package OpenCode loads for this API shape.
    pub fn package(self) -> &'static str {
        match self {
            Engine::OpenAi => "@opencode/ai/providers/openai-compatible",
            Engine::Anthropic => "@opencode/ai/providers/anthropic",
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
    /// Provider entry that serves this model.
    pub provider: &'static str,
    /// Context window written to the OpenCode model entry.
    pub context_limit: u64,
}

/// Output token ceiling written to every model entry. OpenCode uses it to
/// budget a response; the services cap below it when their own limit is lower.
pub const OUTPUT_LIMIT: u64 = 65_536;

#[derive(Clone, Copy, Debug)]
pub struct Declarative {
    /// Provider ID in `opencode.json` and the prefix of every model reference.
    pub name: &'static str,
    pub display_name: &'static str,
    pub engine: Engine,
    /// Appended to the provider's configured base URL.
    pub base_suffix: &'static str,
    pub api_key_env: &'static str,
    pub models: &'static [Model],
}

impl Declarative {
    /// The value written to `providers.<id>.settings.baseURL` and the root of
    /// the key-validation request.
    pub fn base_url(&self) -> String {
        format!(
            "{}{}",
            settings::endpoints()
                .provider_base(self.name)
                .trim_end_matches('/'),
            self.base_suffix
        )
    }

    /// `GET` endpoint used to validate a key. Every supported API exposes its
    /// model list beside the same `/v1` root the requests use.
    pub fn validate_url(&self) -> String {
        format!("{}/models", self.base_url().trim_end_matches('/'))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProviderSpec {
    /// User-facing ID and CLI flag value (`rhai-maas`, `enmaas`).
    pub id: &'static str,
    pub display: &'static str,
    /// OpenCode provider ID shown in the configuration and model reference.
    pub handle: &'static str,
    pub vpn: VpnNeed,
    /// Environment variable that carries this provider's key.
    pub secret: &'static str,
    pub declaratives: &'static [Declarative],
    pub default_provider: &'static str,
    pub default_model: &'static str,
    /// Which provider becomes the default when several are configured. EnMaaS
    /// wins so DeepSeek 4.1 Flash is the first model.
    pub priority: u8,
}

impl ProviderSpec {
    pub fn validate_url(&self) -> String {
        self.declaratives
            .first()
            .map(Declarative::validate_url)
            .unwrap_or_default()
    }

    pub fn key_page(&self) -> String {
        settings::endpoints().key_page(self.id).to_string()
    }

    pub fn help_url(&self) -> String {
        settings::endpoints().help(self.id).to_string()
    }

    pub fn eligibility_url(&self) -> Option<String> {
        settings::endpoints()
            .eligibility(self.id)
            .map(str::to_string)
    }

    pub fn slack(&self) -> Option<String> {
        settings::endpoints().slack(self.id).map(str::to_string)
    }
}

const RHAI_MAAS_MODELS: &[Model] = &[
    Model {
        name: "publishers/prelude-maas/models/glm-53-flash",
        display: "GLM 5.3 Flash",
        provider: "ai-bu",
        context_limit: 200_000,
    },
    Model {
        name: "publishers/prelude-maas/models/qwen38-27b",
        display: "Qwen 3.8 27B",
        provider: "ai-bu",
        context_limit: 200_000,
    },
    Model {
        name: "publishers/prelude-maas/models/llama-4-scout-17b-16e-w4a16",
        display: "Llama 4 Scout 17B",
        provider: "ai-bu",
        context_limit: 200_000,
    },
    Model {
        name: "publishers/prelude-maas/models/nemotron-cascade-2-30b",
        display: "Nemotron Cascade 2 30B",
        provider: "ai-bu",
        context_limit: 200_000,
    },
    Model {
        name: "publishers/prelude-maas/models/gemma4",
        display: "Gemma 4",
        provider: "ai-bu",
        context_limit: 200_000,
    },
    Model {
        name: "publishers/prelude-maas/models/llama-32-3b",
        display: "Llama 3.2 3B",
        provider: "ai-bu",
        context_limit: 200_000,
    },
];

const ENMAAS_MODELS: &[Model] = &[
    Model {
        name: "rits/deepseek-ai/DeepSeek-V4.1-Flash-1017",
        display: "DeepSeek 4.1 Flash",
        provider: "enmaas",
        context_limit: 1_000_000,
    },
    Model {
        name: "rits/zai-org/glm-5-3",
        display: "GLM 5.3",
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
        provider: "enmaas-oai",
        context_limit: 1_000_000,
    },
    Model {
        name: "gemini-3.8-flash",
        display: "Gemini 3.8 Flash",
        provider: "enmaas-oai",
        context_limit: 1_000_000,
    },
    Model {
        name: "gemini-3.7-flash",
        display: "Gemini 3.7 Flash",
        provider: "enmaas-oai",
        context_limit: 1_000_000,
    },
    Model {
        name: "gemini-3.6-flash",
        display: "Gemini 3.6 Flash",
        provider: "enmaas-oai",
        context_limit: 1_000_000,
    },
];

const RHAI_MAAS_DECLARATIVES: &[Declarative] = &[Declarative {
    name: "ai-bu",
    display_name: "Red Hat AI MaaS",
    engine: Engine::OpenAi,
    base_suffix: "",
    api_key_env: "RHAI_MAAS_API_KEY",
    models: RHAI_MAAS_MODELS,
}];

const ENMAAS_DECLARATIVES: &[Declarative] = &[
    Declarative {
        name: "enmaas",
        display_name: "EnMaaS",
        engine: Engine::Anthropic,
        // The Anthropic runtime joins "/messages" to this base, so it needs /v1.
        base_suffix: "/v1",
        api_key_env: "ENMAAS_API_KEY",
        models: ENMAAS_MODELS,
    },
    Declarative {
        name: "enmaas-oai",
        display_name: "EnMaaS (OpenAI compatible)",
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
        handle: "ai-bu",
        vpn: VpnNeed::Off,
        secret: "RHAI_MAAS_API_KEY",
        declaratives: RHAI_MAAS_DECLARATIVES,
        default_provider: "ai-bu",
        default_model: "publishers/prelude-maas/models/glm-53-flash",
        priority: 1,
    },
    ProviderSpec {
        id: "enmaas",
        display: "EnMaaS",
        handle: "enmaas",
        vpn: VpnNeed::On,
        secret: "ENMAAS_API_KEY",
        declaratives: ENMAAS_DECLARATIVES,
        default_provider: "enmaas",
        default_model: "rits/deepseek-ai/DeepSeek-V4.1-Flash-1017",
        priority: 0,
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

/// Find the provider entry and model record for a chosen model name.
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

/// Model reference OpenCode uses: `provider/model`.
pub fn model_ref(provider: &str, model: &str) -> String {
    format!("{provider}/{model}")
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
    fn enmaas_defaults_to_deepseek() {
        let spec = find("enmaas").unwrap();
        assert_eq!(
            spec.default_model,
            "rits/deepseek-ai/DeepSeek-V4.1-Flash-1017"
        );
        assert!(spec.priority < find("rhai-maas").unwrap().priority);
    }

    #[test]
    fn enmaas_shares_one_secret() {
        for d in find("enmaas").unwrap().declaratives {
            assert_eq!(d.api_key_env, "ENMAAS_API_KEY");
        }
    }

    #[test]
    fn every_base_url_ends_with_v1_and_validates_beside_it() {
        for spec in all() {
            for declarative in spec.declaratives {
                let base = declarative.base_url();
                assert!(base.ends_with("/v1"), "{} base {base}", declarative.name);
                assert_eq!(declarative.validate_url(), format!("{base}/models"));
            }
        }
    }

    #[test]
    fn provider_ids_follow_the_opencode_names() {
        assert_eq!(find("rhai-maas").unwrap().declaratives[0].name, "ai-bu");
        assert_eq!(find("enmaas").unwrap().declaratives[0].name, "enmaas");
        assert_eq!(find("enmaas").unwrap().declaratives[1].name, "enmaas-oai");
    }
}
