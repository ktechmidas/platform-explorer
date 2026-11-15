use dpp::state_transition::state_transition_factory::StateTransitionFactory;
use dpp::state_transition::StateTransition;
use dpp::ProtocolError;

pub struct StateTransitionDecoder {
    st_factory: StateTransitionFactory,
}

impl StateTransitionDecoder {
    pub async fn decode(&self, data: Vec<u8>) -> Result<StateTransition, ProtocolError> {
        let array = data.as_slice();

        let result = self.st_factory.create_from_buffer(array);

        return result;
    }

    pub fn new() -> StateTransitionDecoder {
        let st_factory = StateTransitionFactory {};

        return StateTransitionDecoder { st_factory };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::document::Document;
    use crate::processor::psql::handlers::handle_block::decode_block_transaction;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use dpp::serialization::PlatformSerializable;
    use dpp::state_transition::batch_transition::batched_transition::BatchedTransition;
    use dpp::state_transition::batch_transition::document_base_transition::document_base_transition_trait::DocumentBaseTransitionAccessors;
    use dpp::state_transition::batch_transition::document_base_transition::DocumentBaseTransition;
    use dpp::state_transition::batch_transition::BatchTransition;

    #[tokio::test]
    async fn palmatum_453_rejected_erase_retains_exact_signed_bytes_and_result() {
        let f: serde_json::Value = serde_json::from_str(include_str!("../../tests/fixtures/palmatum-453.json")).unwrap();
        let result: crate::models::TDTxResult = serde_json::from_value(f["result"].clone()).unwrap();
        assert_eq!(result.code.unwrap_or(0),10404);
        let encoded = f["txBase64"].as_str().unwrap(); let raw=STANDARD.decode(encoded).unwrap();
        assert_eq!(sha256::digest(&raw).to_uppercase(),f["txSha256"]);
        let st=decode_block_transaction(&StateTransitionDecoder::new(),encoded,453,0).await.unwrap();
        assert_eq!(st.serialize_to_bytes().unwrap(),raw);
        let StateTransition::Batch(BatchTransition::V1(batch))=st else {panic!("expected V1 batch")};
        assert_eq!(batch.transitions.len(),1);
        let BatchedTransition::Document(doc)=batch.transitions[0].clone() else {panic!("expected document")};
        assert!(matches!(doc,dpp::state_transition::batch_transition::batched_transition::DocumentTransition::Erase(_)));
        let projected=Document::from(doc); assert!(projected.deleted);assert!(projected.data.is_none());assert!(projected.revision.is_none());
    }

    #[tokio::test]
    async fn palmatum_340_complete_wire_regression() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!("../../tests/fixtures/palmatum-340.json")).unwrap();
        let result: crate::models::TDTxResult = serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(result.code.unwrap_or(0), 0);
        let encoded = fixture["txBase64"].as_str().unwrap();
        let bytes = STANDARD.decode(encoded).unwrap();
        assert_eq!(bytes.len(), 1036);
        assert_eq!(sha256::digest(&bytes).to_uppercase(), fixture["txSha256"]);
        let transition = decode_block_transaction(&StateTransitionDecoder::new(), encoded, 340, 0).await.unwrap();
        assert_eq!(transition.serialize_to_bytes().unwrap(), bytes);
        assert!(matches!(transition, StateTransition::DataContractCreate(_)));
        assert!(StateTransitionDecoder::new().decode(bytes[..bytes.len()/2].to_vec()).await.is_err());
    }

    #[tokio::test]
    async fn sakura_beta5_contract_485_preserves_complete_wire_data() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/sakura-beta5-contract-485.json"
        )).unwrap();
        let result: crate::models::TDTxResult =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(result.code.unwrap_or(0), 0);
        let encoded = fixture["txBase64"].as_str().unwrap();
        let bytes = STANDARD.decode(encoded).unwrap();
        assert_eq!(bytes.len(), 16456);
        assert_eq!(sha256::digest(&bytes).to_uppercase(), fixture["txSha256"]);
        let transition = decode_block_transaction(&StateTransitionDecoder::new(), encoded, 485, 0)
            .await.unwrap();
        assert_eq!(transition.serialize_to_bytes().unwrap(), bytes);
        let StateTransition::DataContractCreate(transition) = transition else {
            panic!("expected successful data contract creation");
        };
        use dpp::data_contract::serialized_version::DataContractInSerializationFormat;
        use dpp::state_transition::data_contract_create_transition::DataContractCreateTransition;
        let original = match &transition {
            DataContractCreateTransition::V0(v) => &v.data_contract,
            DataContractCreateTransition::V1(v) => &v.data_contract,
        };
        let DataContractInSerializationFormat::V1(original) = original else {
            panic!("expected beta.5 contract V1");
        };
        let original_schema = serde_json::to_value(&original.document_schemas).unwrap();
        let original_tokens = serde_json::to_value(&original.tokens).unwrap();
        assert!(original.tokens.values().any(|token| matches!(token, dpp::data_contract::TokenConfiguration::V1(_))),
            "captured contract exercises beta.5 token-V1 wire layout");
        let contract = crate::entities::data_contract::DataContract::from(transition);
        assert_eq!(contract.schema.as_ref().unwrap(), &original_schema);
        assert_eq!(serde_json::to_value(contract.tokens.as_ref().unwrap()).unwrap(), original_tokens);
        assert!(contract.schema.as_ref().unwrap().as_object().unwrap().len() > 0);
        assert!(StateTransitionDecoder::new().decode(bytes[..bytes.len() / 2].to_vec()).await.is_err());
    }

    fn has_legacy_v1_token(transition: &dpp_legacy_beta2::state_transition::StateTransition) -> bool {
        use dpp_legacy_beta2::data_contract::serialized_version::DataContractInSerializationFormat as Format;
        use dpp_legacy_beta2::state_transition::data_contract_create_transition::DataContractCreateTransition as Create;
        use dpp_legacy_beta2::state_transition::data_contract_update_transition::DataContractUpdateTransition as Update;
        use dpp_legacy_beta2::state_transition::StateTransition as Legacy;
        let format = match transition {
            Legacy::DataContractCreate(Create::V0(v)) => &v.data_contract,
            Legacy::DataContractCreate(Create::V1(v)) => &v.data_contract,
            Legacy::DataContractUpdate(Update::V0(v)) => &v.data_contract,
            _ => return false,
        };
        match format {
            Format::V1(v) => v.tokens.values().any(|t| matches!(t, dpp_legacy_beta2::data_contract::TokenConfiguration::V1(_))),
            Format::V0(_) => false,
        }
    }

    #[tokio::test]
    async fn sakura_tail_7392_through_7505_preserves_every_wire_transaction() {
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/sakura-tail-7392-7505.json"
        ))
        .unwrap();
        let fixtures = fixtures.as_array().unwrap();
        assert_eq!(fixtures.len(), 116);
        for fixture in fixtures {
            let height = fixture["height"].as_i64().unwrap() as i32;
            let index = fixture["index"].as_u64().unwrap() as usize;
            let encoded = fixture["txBase64"].as_str().unwrap();
            let bytes = STANDARD.decode(encoded).unwrap();
            assert_eq!(sha256::digest(&bytes).to_uppercase(), fixture["txSha256"]);
            // This archived chain predates the beta.5 reset. Prove every original
            // byte still roundtrips with its published decoder; the live decoder
            // must not silently reinterpret an incompatible token-V1 layout.
            let legacy = dpp_legacy_beta2::state_transition::state_transition_factory::StateTransitionFactory {}
                .create_from_buffer(&bytes).unwrap();
            assert_eq!(dpp_legacy_beta2::serialization::PlatformSerializable::serialize_to_bytes(&legacy).unwrap(), bytes);
            let current = decode_block_transaction(&StateTransitionDecoder::new(), encoded, height, index).await;
            if has_legacy_v1_token(&legacy) {
                assert!(current.is_err(), "beta.2 token bytes must fail closed at {height}/{index}");
            } else {
                assert_eq!(current.unwrap().serialize_to_bytes().unwrap(), bytes, "wire data at {height}/{index}");
            }
            let result: crate::models::TDTxResult =
                serde_json::from_value(fixture["result"].clone()).unwrap();
            println!(
                "captured {height}/{index} code={} transition={}",
                result.code.unwrap_or(0),
                legacy.state_transition_type()
            );
        }
    }

    #[tokio::test]
    async fn sakura_7392_token_configuration_v1_preserves_pool_and_wire_bytes() {
        use dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
        use dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
        use dpp::data_contract::associated_token::token_configuration_localization::accessors::v0::TokenConfigurationLocalizationV0Getters;
        use dpp::data_contract::TokenConfiguration;
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/sakura-token-config-7392.json"
        ))
        .unwrap();
        let result: crate::models::TDTxResult =
            serde_json::from_value(fixture["result"].clone()).unwrap();
        assert_eq!(
            result.code.unwrap_or(0),
            0,
            "contract creation succeeded on chain"
        );
        let encoded = fixture["txBase64"].as_str().unwrap();
        let bytes = STANDARD.decode(encoded).unwrap();
        assert_eq!(sha256::digest(&bytes).to_uppercase(), fixture["txSha256"]);
        assert!(decode_block_transaction(&StateTransitionDecoder::new(), encoded, 7392, 0).await.is_err(),
            "pre-reset beta.2 token wire must not be interpreted as beta.5");
        let decoded = dpp_legacy_beta2::state_transition::state_transition_factory::StateTransitionFactory {}
            .create_from_buffer(&bytes).unwrap();
        assert_eq!(dpp_legacy_beta2::serialization::PlatformSerializable::serialize_to_bytes(&decoded).unwrap(), bytes);
        let dpp_legacy_beta2::state_transition::StateTransition::DataContractCreate(legacy_transition) = decoded else {
            panic!("expected data contract creation");
        };
        // Published JSON conversion supplies beta.5's default transferable=true.
        // This is a semantic projection test only, never a live wire fallback.
        let legacy_format = match legacy_transition {
            dpp_legacy_beta2::state_transition::data_contract_create_transition::DataContractCreateTransition::V0(v) => v.data_contract,
            dpp_legacy_beta2::state_transition::data_contract_create_transition::DataContractCreateTransition::V1(v) => v.data_contract,
        };
        let dpp_legacy_beta2::data_contract::serialized_version::DataContractInSerializationFormat::V1(legacy_format) = legacy_format else {
            panic!("expected archived V1 contract");
        };
        let format = dpp::data_contract::serialized_version::DataContractInSerializationFormat::V1(
            serde_json::from_value(serde_json::to_value(legacy_format).unwrap()).unwrap()
        );
        // Synthetic envelope exercises the unchanged schema/token projection;
        // no signed fields are claimed to be converted between wire versions.
        let transition = dpp::state_transition::data_contract_create_transition::DataContractCreateTransition::V0(
            dpp::state_transition::data_contract_create_transition::DataContractCreateTransitionV0 {
                data_contract: format, identity_nonce: 0, user_fee_increase: 0,
                signature_public_key_id: 0, signature: Default::default(),
            }
        );
        // The production conversion must retain the complete token configuration,
        // including V1 fields, rather than coerce it to the old V0 representation.
        let contract = crate::entities::data_contract::DataContract::from(transition);
        let tokens = contract.tokens.unwrap();
        assert_eq!(tokens.len(), 1);
        let token = tokens.get(&0).unwrap();
        let TokenConfiguration::V1(pool) = token else {
            panic!("expected V1 token configuration");
        };
        assert!(pool.has_shielded_pool);
        assert_eq!(token.base_supply(), 1_000_000);
        assert_eq!(
            token
                .conventions()
                .localizations()
                .get("en")
                .unwrap()
                .singular_form(),
            "qapool"
        );
        assert_eq!(
            token
                .conventions()
                .localizations()
                .get("en")
                .unwrap()
                .plural_form(),
            "qapools"
        );
        let json = serde_json::to_value(token).unwrap();
        assert_eq!(json["$formatVersion"], "1");
        assert_eq!(json["hasShieldedPool"], true);
        assert!(contract.schema.unwrap().get("n").is_some());
    }

    #[tokio::test]
    async fn sakura_block_325_decodes_v2_and_preserves_transaction_and_document() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/sakura-block-325.json"))
                .unwrap();
        assert_eq!(fixture["protocolVersion"], 14);
        assert_eq!(fixture["transactions"], 1);
        let encoded = fixture["txBase64"][0].as_str().unwrap();
        let bytes = STANDARD.decode(encoded).unwrap();
        assert_eq!(
            sha256::digest(&bytes).to_uppercase(),
            fixture["txSha256"].as_str().unwrap()
        );
        // Use precisely the block handler's decoding path, not a permissive test decoder.
        let state_transition =
            decode_block_transaction(&StateTransitionDecoder::new(), encoded, 325, 0)
                .await
                .unwrap();
        assert_eq!(state_transition.serialize_to_bytes().unwrap(), bytes);
        let StateTransition::Batch(BatchTransition::V1(batch)) = state_transition else {
            panic!("expected batch v1");
        };
        assert_eq!(
            batch.transitions.len(),
            1,
            "no transactions may be discarded"
        );
        let BatchedTransition::Document(document_transition) =
            batch.transitions.into_iter().next().unwrap()
        else {
            panic!("expected document transition");
        };
        let dpp::state_transition::batch_transition::batched_transition::document_transition::DocumentTransition::Create(ref create) = document_transition else { panic!("expected create"); };
        assert!(matches!(create.base(), DocumentBaseTransition::V2(_)));
        // Exercise the production entity conversion used by the document handler.
        let document = Document::from(document_transition);
        assert_eq!(document.document_type_name, "preorder");
        assert!(!document.deleted);
        assert_eq!(document.revision, Some(1));
        assert!(document.data.unwrap().get("saltedDomainHash").is_some());
    }
    #[tokio::test]
    async fn index_only_delete_retains_the_full_property_tuple_and_tombstone() {
        use dpp::state_transition::batch_transition::batched_transition::document_transition::DocumentTransition;
        use dpp::state_transition::batch_transition::batched_transition::document_index_only_delete_transition::v0::DocumentIndexOnlyDeleteTransitionV0;
        use dpp::state_transition::batch_transition::document_create_transition::v0::v0_methods::DocumentCreateTransitionV0Methods;
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/sakura-block-325.json"))
                .unwrap();
        let st = StateTransitionDecoder::new()
            .decode(
                STANDARD
                    .decode(fixture["txBase64"][0].as_str().unwrap())
                    .unwrap(),
            )
            .await
            .unwrap();
        let StateTransition::Batch(BatchTransition::V1(mut batch)) = st else {
            panic!("expected batch");
        };
        let BatchedTransition::Document(DocumentTransition::Create(create)) =
            batch.transitions.remove(0)
        else {
            panic!("expected create");
        };
        let deletion = DocumentIndexOnlyDeleteTransitionV0 {
            base: create.base().clone(),
            data: create.data().clone(),
        };
        let document = Document::from(DocumentTransition::IndexOnlyDelete(deletion.into()));
        assert!(document.deleted);
        assert_eq!(document.document_type_name, "preorder");
        assert!(document.data.unwrap().get("saltedDomainHash").is_some());
        assert_eq!(document.transition_type as i32, 7);
    }
}
