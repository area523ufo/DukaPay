#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, BytesN, Env};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AnchorError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    EpochAlreadyAnchored = 3,
    EmptyEpoch = 4,
}

#[contracttype]
pub enum DataKey {
    Owner,
    Root(u64),
    LeafCount(u64),
}

#[contract]
pub struct AuditAnchor;

#[contractimpl]
impl AuditAnchor {
    const INSTANCE_TTL_THRESHOLD: u32 = 17_280;
    const INSTANCE_TTL_BUMP: u32 = 518_400;
    const PERSISTENT_TTL_THRESHOLD: u32 = 17_280;
    const PERSISTENT_TTL_BUMP: u32 = 518_400;

    fn bump_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(Self::INSTANCE_TTL_THRESHOLD, Self::INSTANCE_TTL_BUMP);
    }

    fn bump_persistent_ttl(env: &Env, key: &DataKey) {
        env.storage().persistent().extend_ttl(
            key,
            Self::PERSISTENT_TTL_THRESHOLD,
            Self::PERSISTENT_TTL_BUMP,
        );
    }

    pub fn init(env: Env, owner: Address) -> Result<(), AnchorError> {
        if env.storage().instance().has(&DataKey::Owner) {
            return Err(AnchorError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Owner, &owner);
        Self::bump_instance_ttl(&env);
        Ok(())
    }

    pub fn anchor(
        env: Env,
        epoch: u64,
        root: BytesN<32>,
        leaf_count: u32,
    ) -> Result<(), AnchorError> {
        let owner: Address = env
            .storage()
            .instance()
            .get(&DataKey::Owner)
            .ok_or(AnchorError::NotInitialized)?;
        owner.require_auth();
        if leaf_count == 0 {
            return Err(AnchorError::EmptyEpoch);
        }
        let root_key = DataKey::Root(epoch);
        if env.storage().persistent().has(&root_key) {
            return Err(AnchorError::EpochAlreadyAnchored);
        }
        let leaf_key = DataKey::LeafCount(epoch);
        env.storage().persistent().set(&root_key, &root);
        Self::bump_persistent_ttl(&env, &root_key);
        env.storage().persistent().set(&leaf_key, &leaf_count);
        Self::bump_persistent_ttl(&env, &leaf_key);
        Self::bump_instance_ttl(&env);
        env.events().publish(
            (soroban_sdk::Symbol::new(&env, "auditroot"), epoch),
            (root, leaf_count),
        );
        Ok(())
    }

    pub fn get_root(env: Env, epoch: u64) -> Option<BytesN<32>> {
        let key = DataKey::Root(epoch);
        let root = env.storage().persistent().get(&key);
        if root.is_some() {
            Self::bump_persistent_ttl(&env, &key);
        }
        root
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke},
        Address, BytesN, Env, IntoVal,
    };

    #[test]
    fn owner_can_anchor_each_epoch_once_and_extends_ttl() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| {
            li.sequence_number = 100;
            li.min_persistent_entry_ttl = 10;
            li.min_temp_entry_ttl = 10;
            li.max_entry_ttl = 1_000_000;
        });
        let contract_id = env.register(AuditAnchor, ());
        let client = AuditAnchorClient::new(&env, &contract_id);
        let owner = Address::generate(&env);
        client.init(&owner);
        let root = BytesN::from_array(&env, &[7; 32]);
        client.anchor(&1, &root, &3);
        assert_eq!(client.get_root(&1), Some(root));
        assert_eq!(
            client.try_anchor(&1, &BytesN::from_array(&env, &[8; 32]), &3),
            Err(Ok(AnchorError::EpochAlreadyAnchored))
        );

        env.as_contract(&contract_id, || {
            let root_ttl = env.storage().persistent().get_ttl(&DataKey::Root(1));
            let count_ttl = env.storage().persistent().get_ttl(&DataKey::LeafCount(1));
            assert!(root_ttl >= AuditAnchor::PERSISTENT_TTL_BUMP);
            assert!(count_ttl >= AuditAnchor::PERSISTENT_TTL_BUMP);
        });
    }

    #[test]
    fn anchor_without_owner_auth_fails() {
        let env = Env::default();
        let contract_id = env.register(AuditAnchor, ());
        let client = AuditAnchorClient::new(&env, &contract_id);
        let owner = Address::generate(&env);
        let attacker = Address::generate(&env);
        client.init(&owner);

        let root = BytesN::from_array(&env, &[1; 32]);
        assert!(client.try_anchor(&1, &root, &5).is_err());

        env.mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "anchor",
                args: (1u64, root.clone(), 5u32).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        assert!(client.try_anchor(&1, &root, &5).is_err());
    }

    #[test]
    fn anchor_rejects_uninitialized_empty_epoch_and_double_init() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(AuditAnchor, ());
        let client = AuditAnchorClient::new(&env, &contract_id);
        let root = BytesN::from_array(&env, &[3; 32]);

        assert_eq!(
            client.try_anchor(&1, &root, &1),
            Err(Ok(AnchorError::NotInitialized))
        );

        let owner = Address::generate(&env);
        client.init(&owner);
        assert_eq!(
            client.try_init(&owner),
            Err(Ok(AnchorError::AlreadyInitialized))
        );

        assert_eq!(
            client.try_anchor(&1, &root, &0),
            Err(Ok(AnchorError::EmptyEpoch))
        );
    }
}

