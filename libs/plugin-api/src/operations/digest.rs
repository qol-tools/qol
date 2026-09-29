use super::Operation;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};

impl Operation {
    pub fn declaration_digest(&self) -> Result<String, serde_json::Error> {
        Ok(URL_SAFE_NO_PAD.encode(Sha256::digest(self.declaration_bytes()?)))
    }

    fn declaration_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(&(
            "qol-operation-declaration-v1",
            &self.key,
            &self.plugin_id,
            &self.description,
            &self.tool_description,
            &self.input,
            &self.runtime_args,
            &self.action_kind,
            self.agent_tool,
            &self.peer,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ActionType, PluginId, PluginUid};
    use crate::operations::{OperationKey, OperationKind};

    #[test]
    fn operation_declaration_encoding_is_frozen_and_exposure_changes_digest() {
        let mut operation = Operation {
            key: OperationKey::new(
                PluginUid::new("fixture-uid"),
                OperationKind::Action,
                "count",
            ),
            plugin_id: PluginId::new("qol-fixture"),
            description: "Count".into(),
            tool_description: None,
            input: None,
            runtime_args: Some(vec!["count".into()]),
            action_kind: Some(ActionType::Run),
            agent_tool: false,
            peer: Some(qol_config::contract::PeerExposure {
                replay: qol_config::contract::PeerReplay::Idempotent,
            }),
        };
        let frozen = br#"["qol-operation-declaration-v1",{"identity":{"scope":"stable","value":"fixture-uid"},"kind":"action","name":"count"},"qol-fixture","Count",null,null,["count"],"run",false,{"replay":"idempotent"}]"#;
        assert_eq!(operation.declaration_bytes().unwrap(), frozen);
        let digest = operation.declaration_digest().unwrap();
        assert_eq!(digest, URL_SAFE_NO_PAD.encode(Sha256::digest(frozen)));
        operation.peer = None;
        assert_ne!(operation.declaration_digest().unwrap(), digest);
        operation.runtime_args = Some(vec!["other".into()]);
        assert_ne!(operation.declaration_digest().unwrap(), digest);
    }
}
