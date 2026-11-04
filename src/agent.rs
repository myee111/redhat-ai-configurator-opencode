//! The OpenCode primary agent the installer selects as the default.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Agent {
    Build,
    Plan,
}

impl Agent {
    pub fn as_str(self) -> &'static str {
        match self {
            Agent::Build => "build",
            Agent::Plan => "plan",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Agent::Build => "Build - edits files and runs tools",
            Agent::Plan => "Plan - reads and plans, changes nothing",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "build" | "default" => Some(Agent::Build),
            "plan" | "readonly" | "read-only" => Some(Agent::Plan),
            _ => None,
        }
    }

    pub const ALL: [Agent; 2] = [Agent::Build, Agent::Plan];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_friendly_names() {
        assert_eq!(Agent::parse("build"), Some(Agent::Build));
        assert_eq!(Agent::parse("PLAN"), Some(Agent::Plan));
        assert_eq!(Agent::parse("read-only"), Some(Agent::Plan));
        assert_eq!(Agent::parse("nope"), None);
    }
}
