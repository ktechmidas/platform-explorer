//! Public encrypted-entry history, not decrypted documents or inferred owners.
use super::super::{PSQLProcessor, ProcessorError};
use deadpool_postgres::Transaction;
use dpp::platform_value::string_encoding::Encoding::Base58;
use dpp::state_transition::private_document_create_from_shielded_pool_transition::PrivateDocumentCreateFromShieldedPoolTransition;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;

fn entries(st: &PrivateDocumentCreateFromShieldedPoolTransition) -> Value {
    let PrivateDocumentCreateFromShieldedPoolTransition::V0(st) = st;
    // Preserve ordering and every ciphertext byte; no normalization/decryption.
    serde_json::json!(st.entries.iter().map(|e| STANDARD.encode(e.as_slice())).collect::<Vec<_>>())
}
impl PSQLProcessor {
    pub async fn handle_private_documents(&self, st: PrivateDocumentCreateFromShieldedPoolTransition,
        hash: String, height: i32, index: usize, tx: &Transaction<'_>) -> Result<(), ProcessorError> {
        let encrypted_entries = entries(&st);
        let PrivateDocumentCreateFromShieldedPoolTransition::V0(st) = st;
        let contract = st.data_contract_id.to_string(Base58);
        let value_balance = st.value_balance.to_string();
        tx.execute("INSERT INTO private_document_events(state_transition_hash,contract_identifier,document_type_name,encrypted_entries,value_balance) VALUES($1,$2,$3,$4,$5::text::numeric)",
            &[&hash,&contract,&st.document_type_name,&encrypted_entries,&value_balance]).await.map_err(|e| ProcessorError::TransactionError {
                height,index,stage:"private document event projection",detail:e.to_string()
            })?;
        self.handle_data_contract_transition(Some(hash),st.data_contract_id,tx).await;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use dpp::serialization::PlatformSerializable;
    use dpp::state_transition::StateTransition;
    use crate::decoder::decoder::StateTransitionDecoder;
    #[tokio::test]
    async fn actual_679_private_entries_are_exact_ordered_ciphertext() {
        let f:Value=serde_json::from_str(include_str!("../../../../tests/fixtures/palmatum-679.json")).unwrap();
        let result:crate::models::TDTxResult=serde_json::from_value(f["result"].clone()).unwrap();assert_eq!(result.code.unwrap_or(0),0);
        let raw=STANDARD.decode(f["txBase64"].as_str().unwrap()).unwrap();assert_eq!(sha256::digest(&raw).to_uppercase(),f["txSha256"]);
        let decoded=StateTransitionDecoder::new().decode(raw.clone()).await.unwrap();assert_eq!(decoded.serialize_to_bytes().unwrap(),raw);
        let StateTransition::PrivateDocumentCreateFromShieldedPool(st)=decoded else {panic!("expected private document create")};
        let projected=entries(&st);let PrivateDocumentCreateFromShieldedPoolTransition::V0(st)=st;
        assert!(!st.entries.is_empty());assert_eq!(projected.as_array().unwrap().len(),st.entries.len());
        for (original, projected) in st.entries.iter().zip(projected.as_array().unwrap()) {assert_eq!(STANDARD.decode(projected.as_str().unwrap()).unwrap(),original.as_slice());}
        assert!(st.value_balance>0);
    }
}
