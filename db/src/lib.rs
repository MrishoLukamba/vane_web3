#![allow(unused_imports)]
#![allow(unused)]
extern crate alloc;

mod db;

#[cfg(test)]
mod db_tests;

use crate::db::read_filters::BoolFilter;
use crate::db::transactions_data::{UniqueWhereParam, WhereParam};
use crate::db::{
    new_client_with_url, peer,
    read_filters::{BigIntFilter, BytesFilter, IntFilter},
    transaction, transactions_data, user_account, PrismaClient, PrismaClientBuilder,
};
use alloc::sync::Arc;
use anyhow::anyhow;
use codec::{Decode, Encode};
use hex;
use log::{debug, error, trace, warn};
use primitives::data_structure::{ChainSupported, DbTxStateMachine, PeerRecord, UserAccount};
use prisma_client_rust::{query_core::RawQuery, BatchItem, Direction, PrismaValue, Raw};
use serde::{Deserialize, Serialize};

/// Handling connection and interaction with the database
pub struct DbWorker {
    db: Arc<PrismaClient>,
}

const SERVER_DATA_ID: i32 = 100;

impl DbWorker {
    pub async fn initialize_db_client(file_url: &str) -> Result<Self, anyhow::Error> {
        let url = format!("file:{}", file_url);
        let client = new_client_with_url(&url).await?;
        let client = Arc::new(client);
        let data_option = client
            .transactions_data()
            .find_first(vec![WhereParam::Id(IntFilter::Equals(SERVER_DATA_ID))])
            .exec()
            .await?;
        match data_option {
            Some(_data) => {}
            None => {
                client
                    .transactions_data()
                    .create(SERVER_DATA_ID, 0, 0, vec![])
                    .exec()
                    .await?;
            }
        }
        Ok(Self { db: client })
    }

    pub async fn set_user_account(&self, user: UserAccount) -> Result<(), anyhow::Error> {
        self.db
            .user_account()
            .create(
                user.user_name,
                user.account_id,
                user.network.encode(),
                Default::default(),
            )
            .exec()
            .await?;
        Ok(())
    }

    // get all related network id accounts
    pub async fn get_user_accounts(
        &self,
        network: ChainSupported,
    ) -> Result<Vec<user_account::Data>, anyhow::Error> {
        let accounts = self
            .db
            .user_account()
            .find_many(vec![user_account::WhereParam::NetworkId(
                BytesFilter::Equals(network.encode()),
            )])
            .exec()
            .await?;
        Ok(accounts)
    }

    pub async fn update_success_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error> {
        let tx = self
            .db
            .transaction()
            .create(
                tx_state.tx_hash,
                tx_state.amount as i64,
                tx_state.network.encode(),
                tx_state.success,
                Default::default(),
            )
            .exec()
            .await?;

        self.db
            .transactions_data()
            .update(
                transactions_data::id::equals(SERVER_DATA_ID),
                vec![transactions_data::success_value::increment(
                    tx_state.amount as i64,
                )],
            )
            .exec()
            .await?;

        Ok(())
    }

    pub async fn update_failed_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error> {
        let tx = self
            .db
            .transaction()
            .create(
                tx_state.tx_hash,
                tx_state.amount as i64,
                tx_state.network.encode(),
                tx_state.success,
                Default::default(),
            )
            .exec()
            .await?;

        self.db
            .transactions_data()
            .update(
                transactions_data::id::equals(SERVER_DATA_ID),
                vec![transactions_data::failed_value::increment(
                    tx_state.amount as i64,
                )],
            )
            .exec()
            .await?;
        Ok(())
    }

    pub async fn get_failed_txs(&self) -> Result<Vec<transaction::Data>, anyhow::Error> {
        let failed_txs = self
            .db
            .transaction()
            .find_many(vec![transaction::WhereParam::Status(BoolFilter::Equals(
                false,
            ))])
            .exec()
            .await?;
        Ok(failed_txs)
    }

    pub async fn get_success_txs(&self) -> Result<Vec<transaction::Data>, anyhow::Error> {
        let success_txs = self
            .db
            .transaction()
            .find_many(vec![transaction::WhereParam::Status(BoolFilter::Equals(
                true,
            ))])
            .exec()
            .await?;
        Ok(success_txs)
    }

    pub async fn get_total_value_success(&self) -> Result<u64, anyhow::Error> {
        let main_data = self
            .db
            .transactions_data()
            .find_unique(transactions_data::id::equals(SERVER_DATA_ID))
            .exec()
            .await?
            .ok_or(anyhow!(
                "Main Data not found, shouldnt happen must initailize"
            ))?;
        let success_value = main_data.success_value as u64;
        Ok(success_value)
    }

    pub async fn get_total_value_failed(&self) -> Result<u64, anyhow::Error> {
        let main_data = self
            .db
            .transactions_data()
            .find_unique(transactions_data::id::equals(SERVER_DATA_ID))
            .exec()
            .await?
            .ok_or(anyhow!(
                "Main Data not found, shouldnt happen must initailize"
            ))?;
        let failed_value = main_data.failed_value as u64;
        Ok(failed_value)
    }

    pub async fn record_peer(&self, peer_record: PeerRecord) -> Result<(), anyhow::Error> {
        self.db
            .peer()
            .create(
                peer_record.peer_address,
                peer_record.network.encode(),
                Default::default(),
            )
            .exec()
            .await?;
        Ok(())
    }

    // get peer by account id
    pub async fn get_saved_peer(&self, account_id: Vec<u8>) -> Result<peer::Data, anyhow::Error> {
        let peer_data = self
            .db
            .peer()
            .find_first(vec![peer::WhereParam::PeerId(BytesFilter::Equals(
                account_id,
            ))])
            .exec()
            .await?
            .ok_or(anyhow!("Peer Not found in DB"))?;
        Ok(peer_data)
    }
}

// Type convertions
impl From<peer::Data> for PeerRecord {
    fn from(value: peer::Data) -> Self {
        let decoded_network: ChainSupported = Decode::decode(&mut &value.network_id[..]).unwrap();
        Self {
            peer_address: value.peer_id,
            network: decoded_network,
        }
    }
}

impl From<user_account::Data> for UserAccount {
    fn from(value: user_account::Data) -> Self {
        let decoded_network: ChainSupported = Decode::decode(&mut &value.network_id[..]).unwrap();
        Self {
            user_name: value.username,
            account_id: value.account_id,
            network: decoded_network,
        }
    }
}

impl From<transaction::Data> for DbTxStateMachine {
    fn from(value: transaction::Data) -> Self {
        let decoded_network: ChainSupported = Decode::decode(&mut &value.network[..]).unwrap();
        Self {
            tx_hash: value.tx_hash,
            amount: value.value as u64,
            network: decoded_network,
            success: value.status,
        }
    }
}