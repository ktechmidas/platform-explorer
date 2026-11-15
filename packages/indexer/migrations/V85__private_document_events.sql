-- Append-only public encrypted-entry history; original transition retains all
-- proof/action bytes. Do not invent plaintext documents, IDs or owner identity.
CREATE TABLE private_document_events (
    state_transition_hash char(64) PRIMARY KEY REFERENCES state_transitions(hash),
    contract_identifier varchar(44) NOT NULL,
    document_type_name text NOT NULL,
    encrypted_entries jsonb NOT NULL,
    value_balance numeric(20,0) NOT NULL
);
CREATE INDEX private_document_events_contract ON private_document_events(contract_identifier);
