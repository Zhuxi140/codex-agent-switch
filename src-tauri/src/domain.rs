use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandAuth {
    command: PathBuf,
    credential_id: String,
}

impl CommandAuth {
    pub(crate) fn new(
        command: impl Into<PathBuf>,
        credential_id: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let command = command.into();
        let credential_id = credential_id.into();

        if !command.is_absolute() {
            return Err(DomainError::InvalidField("auth.command"));
        }
        if credential_id.trim().is_empty() {
            return Err(DomainError::InvalidField("credential_id"));
        }

        Ok(Self {
            command,
            credential_id,
        })
    }

    pub(crate) fn command(&self) -> &PathBuf {
        &self.command
    }

    pub(crate) fn credential_id(&self) -> &str {
        &self.credential_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponsesProvider {
    key: String,
    display_name: String,
    base_url: String,
    auth: CommandAuth,
}

impl ResponsesProvider {
    pub(crate) fn new(
        key: impl Into<String>,
        display_name: impl Into<String>,
        base_url: impl Into<String>,
        auth: CommandAuth,
    ) -> Result<Self, DomainError> {
        let key = key.into();
        let display_name = display_name.into();
        let base_url = base_url.into();

        if !is_cas_provider_key(&key) {
            return Err(DomainError::InvalidField("provider.key"));
        }
        if display_name.trim().is_empty() {
            return Err(DomainError::InvalidField("provider.display_name"));
        }
        if !base_url.starts_with("https://") {
            return Err(DomainError::InvalidField("provider.base_url"));
        }

        Ok(Self {
            key,
            display_name,
            base_url,
            auth,
        })
    }

    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    pub(crate) fn display_name(&self) -> &str {
        &self.display_name
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn auth(&self) -> &CommandAuth {
        &self.auth
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Model {
    id: String,
    provider_key: String,
    catalog_path: PathBuf,
}

impl Model {
    pub(crate) fn new(
        id: impl Into<String>,
        provider_key: impl Into<String>,
        catalog_path: impl Into<PathBuf>,
    ) -> Result<Self, DomainError> {
        let id = id.into();
        let provider_key = provider_key.into();
        let catalog_path = catalog_path.into();

        if id.trim().is_empty() {
            return Err(DomainError::InvalidField("model.id"));
        }
        if !is_cas_provider_key(&provider_key) {
            return Err(DomainError::InvalidField("model.provider_key"));
        }
        if !catalog_path.is_absolute() {
            return Err(DomainError::InvalidField("model.catalog_path"));
        }

        Ok(Self {
            id,
            provider_key,
            catalog_path,
        })
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn provider_key(&self) -> &str {
        &self.provider_key
    }

    pub(crate) fn catalog_path(&self) -> &PathBuf {
        &self.catalog_path
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Agent {
    key: String,
    name: String,
    description: String,
    developer_instructions: String,
}

impl Agent {
    pub(crate) fn new(
        key: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        developer_instructions: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let key = key.into();
        let name = name.into();
        let description = description.into();
        let developer_instructions = developer_instructions.into();

        if !is_agent_key(&key) {
            return Err(DomainError::InvalidField("agent.key"));
        }
        if name.trim().is_empty() {
            return Err(DomainError::InvalidField("agent.name"));
        }
        if description.trim().is_empty() {
            return Err(DomainError::InvalidField("agent.description"));
        }
        if developer_instructions.trim().is_empty() {
            return Err(DomainError::InvalidField("agent.developer_instructions"));
        }

        Ok(Self {
            key,
            name,
            description,
            developer_instructions,
        })
    }

    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn description(&self) -> &str {
        &self.description
    }

    pub(crate) fn developer_instructions(&self) -> &str {
        &self.developer_instructions
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReasoningEffort {
    Low,
    Medium,
    High,
    XHigh,
}

impl ReasoningEffort {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BaseBinding {
    agent_key: String,
    model_id: String,
    reasoning_effort: ReasoningEffort,
}

impl BaseBinding {
    pub(crate) fn new(
        agent_key: impl Into<String>,
        model_id: impl Into<String>,
        reasoning_effort: ReasoningEffort,
    ) -> Result<Self, DomainError> {
        let agent_key = agent_key.into();
        let model_id = model_id.into();

        if !is_agent_key(&agent_key) {
            return Err(DomainError::InvalidField("binding.agent_key"));
        }
        if model_id.trim().is_empty() {
            return Err(DomainError::InvalidField("binding.model_id"));
        }

        Ok(Self {
            agent_key,
            model_id,
            reasoning_effort,
        })
    }

    pub(crate) fn agent_key(&self) -> &str {
        &self.agent_key
    }

    pub(crate) fn model_id(&self) -> &str {
        &self.model_id
    }

    pub(crate) fn reasoning_effort(&self) -> ReasoningEffort {
        self.reasoning_effort
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DomainError {
    InvalidField(&'static str),
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(formatter, "invalid field: {field}"),
        }
    }
}

impl std::error::Error for DomainError {}

fn is_cas_provider_key(value: &str) -> bool {
    value
        .strip_prefix("cas_")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(is_key_byte))
}

fn is_agent_key(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(is_key_byte)
}

fn is_key_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_cas_provider_key() {
        let auth = CommandAuth::new(
            r"C:\Program Files\Codex Agent Switch\bin\cas-helper.exe",
            "credential-1",
        )
        .unwrap();

        let result =
            ResponsesProvider::new("deepseek", "DeepSeek", "https://api.deepseek.com/", auth);

        assert_eq!(
            result.unwrap_err(),
            DomainError::InvalidField("provider.key")
        );
    }
}
