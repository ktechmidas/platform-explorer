use crate::decoder::decoder::StateTransitionDecoder;
use crate::entities::block::Block;
use crate::entities::validator::Validator;
use crate::processor::psql::{PSQLProcessor, ProcessorError};
use base64::engine::general_purpose;
use base64::Engine;
use dpp::state_transition::StateTransition;

/// Decode before opening the SQL transaction. A bad transaction must not panic,
/// be skipped, or leave a partially indexed block behind.
pub(crate) async fn decode_block_transaction(
    decoder: &StateTransitionDecoder,
    data: &str,
    height: i32,
    index: usize,
) -> Result<StateTransition, ProcessorError> {
    let bytes = general_purpose::STANDARD.decode(data).map_err(|error| {
        ProcessorError::TransactionError {
            height,
            index,
            stage: "base64 decode",
            detail: error.to_string(),
        }
    })?;
    let transition = decoder
        .decode(bytes)
        .await
        .map_err(|error| ProcessorError::TransactionError {
            height,
            index,
            stage: "state transition decode",
            detail: error.to_string(),
        })?;
    Ok(transition)
}

// Reject unsupported projection shapes before SQL begins. Never skip a signed
// transition or reinterpret a new wire variant as an old one.
fn validate_projection(st: &StateTransition) -> Result<(), &'static str> {
    use dpp::data_contract::serialized_version::DataContractInSerializationFormat as Format;
    use dpp::state_transition::data_contract_create_transition::DataContractCreateTransition as Create;
    use dpp::state_transition::data_contract_update_transition::DataContractUpdateTransition as Update;
    use dpp::state_transition::masternode_vote_transition::accessors::MasternodeVoteTransitionAccessorsV0;
    use dpp::voting::{votes::{Vote, resource_vote::ResourceVote}, vote_polls::VotePoll};
    match st {
        StateTransition::DataContractCreate(Create::V0(v)) if matches!(v.data_contract, Format::V2(_)) => Err("contract V2 projection unsupported"),
        StateTransition::DataContractCreate(Create::V1(v)) if matches!(v.data_contract, Format::V2(_)) => Err("contract V2 projection unsupported"),
        StateTransition::DataContractUpdate(Update::V0(v)) if matches!(v.data_contract, Format::V2(_)) => Err("contract V2 projection unsupported"),
        StateTransition::MasternodeVote(v) => match v.vote() {
            Vote::YesNoVote(_) => Err("yes/no vote projection unsupported"),
            Vote::ResourceVote(ResourceVote::V0(v)) if matches!(v.vote_poll, VotePoll::YesNoVotePoll(_)) => Err("yes/no poll projection unsupported"),
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

impl PSQLProcessor {
    pub async fn handle_block(
        &self,
        block: Block,
        validators: Vec<Validator>,
    ) -> Result<(), ProcessorError> {
        let height = block.header.height;
        if self.dao.get_block_header_by_height(height).await?.is_some() {
            println!("Block at the height {height} has been already processed");
            return Ok(());
        }

        let mut transitions = Vec::with_capacity(block.txs.len());
        for (index, tx) in block.txs.iter().enumerate() {
            let decoded = decode_block_transaction(&self.decoder, &tx.data, height, index).await?;
            // Rejected chain transactions retain their raw bytes/result but do
            // not mutate projections. Gate only successful execution shapes.
            if matches!(tx.status, crate::models::TransactionStatus::SUCCESS) {
                validate_projection(&decoded).map_err(|detail| ProcessorError::TransactionError {
                    height, index, stage: "protocol 15 projection", detail: detail.into(),
                })?;
            }
            transitions.push(decoded);
        }

        let mut client = self.dao.connection_pool.get().await?;
        let sql_transaction =
            client
                .transaction()
                .await
                .map_err(|error| ProcessorError::BlockDatabaseError {
                    height,
                    operation: "begin",
                    detail: error.to_string(),
                })?;

        // Initialization and block 1 are atomic too: a failed first block must
        // not commit initial rows separately and duplicate them on retry.
        if height == 1 {
            self.handle_init_chain(height, &sql_transaction).await?;
        } else if !self.system_contracts_seeded.get() {
            self.ensure_system_contracts(height, &sql_transaction)
                .await?;
        }

        for validator in validators {
            self.handle_validator(validator, &sql_transaction).await?;
        }
        let block_hash = self.dao.create_block(block.header, &sql_transaction).await;

        println!("Processing block at height {height}");
        for (index, (state_transition, tx)) in transitions.into_iter().zip(block.txs).enumerate() {
            self.handle_st(
                block_hash.clone(),
                height,
                index as u32,
                state_transition,
                tx,
                &sql_transaction,
            )
            .await?;
        }

        sql_transaction
            .commit()
            .await
            .map_err(|error| ProcessorError::BlockDatabaseError {
                height,
                operation: "commit",
                detail: error.to_string(),
            })?;
        self.system_contracts_seeded.set(true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn erase_shape_projects_tombstone_in_both_batch_versions() {
        use dpp::identifier::Identifier;
        use dpp::serialization::PlatformSerializable;
        use dpp::state_transition::batch_transition::{BatchTransition, BatchTransitionV0, BatchTransitionV1};
        use dpp::state_transition::batch_transition::batched_transition::{BatchedTransition, DocumentTransition};
        use dpp::state_transition::batch_transition::batched_transition::document_erase_transition::{DocumentEraseTransition, DocumentEraseTransitionV0};
        use dpp::state_transition::batch_transition::document_base_transition::{DocumentBaseTransition, v0::DocumentBaseTransitionV0};
        let erase = DocumentTransition::Erase(DocumentEraseTransition::V0(DocumentEraseTransitionV0 {
            base: DocumentBaseTransition::V0(DocumentBaseTransitionV0 {
                id: Identifier::new([3;32]), identity_contract_nonce: 1,
                document_type_name: "note".into(), data_contract_id: Identifier::new([4;32]),
            }),
        }));
        for batch in [BatchTransition::V0(BatchTransitionV0 { transitions: vec![erase.clone()], ..Default::default() }),
                      BatchTransition::V1(BatchTransitionV1 { transitions: vec![BatchedTransition::Document(erase.clone())], ..Default::default() })] {
            let st = StateTransition::Batch(batch);
            assert_eq!(validate_projection(&st), Ok(()));
            let projected = crate::entities::document::Document::from(erase.clone());
            assert!(projected.deleted); assert!(projected.data.is_none());
            assert_eq!(projected.identifier, Identifier::new([3;32]));
            assert_eq!(projected.data_contract_identifier, Identifier::new([4;32]));
            let raw = st.serialize_to_bytes().unwrap();
            let encoded = general_purpose::STANDARD.encode(raw);
            let decoded = decode_block_transaction(&StateTransitionDecoder::new(), &encoded, 999, 2).await.unwrap();
            assert_eq!(decoded.serialize_to_bytes().unwrap(), st.serialize_to_bytes().unwrap());
        }
    }

    #[tokio::test]
    async fn invalid_base64_returns_block_and_transaction_context() {
        let error = decode_block_transaction(&StateTransitionDecoder::new(), "!not-base64", 325, 7)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ProcessorError::TransactionError {
                height: 325,
                index: 7,
                stage: "base64 decode",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn invalid_state_transition_returns_context_without_panicking() {
        let error = decode_block_transaction(&StateTransitionDecoder::new(), "AA==", 326, 2)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ProcessorError::TransactionError {
                height: 326,
                index: 2,
                stage: "state transition decode",
                ..
            }
        ));
    }
}
