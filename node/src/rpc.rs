//! A collection of node-specific RPC methods.
//! Substrate provides the `sc-rpc` crate, which defines the core RPC layer
//! used by Substrate nodes. This file extends those RPC definitions with
//! capabilities that are specific to this project's runtime configuration.

#![warn(missing_docs)]

use crate::primitives as node_primitives;
use std::{collections::BTreeMap, sync::Arc};

use jsonrpsee::RpcModule;
pub type RpcExtension = jsonrpsee::RpcModule<()>;
use node_primitives::{AccountId, Balance, Block, BlockNumber, Hash, Index};

// Substrate
use cessc_consensus_rrsc::{Epoch, RRSCConfiguration};
use cessp_consensus_rrsc::RRSCApi;
use grandpa::{
	FinalityProofProvider, GrandpaJustificationStream, SharedAuthoritySet, SharedVoterState,
};
use sc_client_api::{
	backend::{AuxStore, Backend, StateBackend, StorageProvider},
	client::BlockchainEvents,
};
use sc_consensus_epochs::SharedEpochChanges;
use sc_network::NetworkService;
use sc_rpc::SubscriptionTaskExecutor;
pub use sc_rpc_api::DenyUnsafe;
use sc_service::TransactionPool;
use sc_transaction_pool::{ChainApi, Pool};
use sp_api::ProvideRuntimeApi;
use sp_block_builder::BlockBuilder;
use sp_blockchain::{Error as BlockChainError, HeaderBackend, HeaderMetadata};
use sp_consensus::SelectChain;
use sp_keystore::SyncCryptoStorePtr;
use sp_runtime::traits::{BlakeTwo256, Block as BlockT};

// Frontier
use fc_rpc::{
	EthBlockDataCacheTask, OverrideHandle, RuntimeApiStorageOverride, SchemaV1Override,
	SchemaV2Override, SchemaV3Override, StorageOverride,
};
use fc_rpc_core::types::{FeeHistoryCache, FilterPool};
use fp_storage::EthereumStorageSchema;

#[derive(Clone, Debug, clap::Parser)]
pub struct EthConfiguration {
	/// Maximum number of logs in a query.
	#[arg(long, default_value = "10000")]
	pub max_past_logs: u32,

	/// Maximum fee history cache size.
	#[arg(long, default_value = "2048")]
	pub fee_history_limit: u64,

	#[arg(long)]
	pub enable_dev_signer: bool,

	/// Maximueth_statuses_cachem allowed gas limit will be `block.gas_limit *
	/// execute_gas_limit_multiplier` when using eth_call/eth_estimateGas.
	#[arg(long, default_value = "10")]
	pub execute_gas_limit_multiplier: u64,
}
/// Full client dependencies.
pub struct FullDeps<C, P, A: ChainApi, CT, BE> {
	/// The client instance to use.
	pub client: Arc<C>,
	/// Transaction pool instance.
	pub pool: Arc<P>,
	/// Whether to deny unsafe calls
	pub deny_unsafe: DenyUnsafe,
	/// Graph pool instance.
	pub graph: Arc<Pool<A>>,
	/// Ethereum transaction converter.
	pub converter: Option<CT>,
	/// The Node authority flag
	pub is_authority: bool,
	/// Whether to enable dev signer
	pub enable_dev_signer: bool,
	/// Network service
	pub network: Arc<NetworkService<Block, Hash>>,
	/// EthFilterApi pool.
	pub filter_pool: Option<FilterPool>,
	/// Frontier Backend.
	pub frontier_backend: Arc<fc_db::Backend<Block>>,
	/// Backend.
	pub backend: Arc<BE>,
	/// Maximum number of logs in a query.
	pub max_past_logs: u32,
	/// Maximum fee history cache size.
	pub fee_history_limit: u64,
	/// Fee history cache.
	pub fee_history_cache: FeeHistoryCache,
	/// Ethereum data access overrides.
	pub overrides: Arc<OverrideHandle<Block>>,
	/// Cache for Ethereum block data.
	pub block_data_cache: Arc<EthBlockDataCacheTask<Block>>,
	/// Maximum allowed gas limit will be ` block.gas_limit * execute_gas_limit_multiplier` when
	/// using eth_call/eth_estimateGas.
	pub execute_gas_limit_multiplier: u64,
}

pub fn overrides_handle<C, BE>(client: Arc<C>) -> Arc<OverrideHandle<Block>>
where
	C: ProvideRuntimeApi<Block> + StorageProvider<Block, BE> + AuxStore,
	C: HeaderBackend<Block> + HeaderMetadata<Block, Error = BlockChainError>,
	C: Send + Sync + 'static,
	C::Api: sp_api::ApiExt<Block>
		+ fp_rpc::EthereumRuntimeRPCApi<Block>
		+ fp_rpc::ConvertTransactionRuntimeApi<Block>,
	BE: Backend<Block> + 'static,
	BE::State: StateBackend<BlakeTwo256>,
{
	let mut overrides_map = BTreeMap::new();
	overrides_map.insert(
		EthereumStorageSchema::V1,
		Box::new(SchemaV1Override::new(client.clone()))
			as Box<dyn StorageOverride<_> + Send + Sync>,
	);
	overrides_map.insert(
		EthereumStorageSchema::V2,
		Box::new(SchemaV2Override::new(client.clone()))
			as Box<dyn StorageOverride<_> + Send + Sync>,
	);
	overrides_map.insert(
		EthereumStorageSchema::V3,
		Box::new(SchemaV3Override::new(client.clone()))
			as Box<dyn StorageOverride<_> + Send + Sync>,
	);

	Arc::new(OverrideHandle {
		schemas: overrides_map,
		fallback: Box::new(RuntimeApiStorageOverride::new(client.clone())),
	})
}

/// Instantiate all full RPC extensions.
pub fn create_full<C, P, B, BE, A, CT>(
	deps: FullDeps<C, P, A, CT, BE>,
	subscription_task_executor: SubscriptionTaskExecutor,
	backend: Arc<B>,
) -> Result<RpcModule<()>, Box<dyn std::error::Error + Send + Sync>>
where
	BE: Backend<Block> + 'static,
	BE::State: StateBackend<BlakeTwo256>,
	C: ProvideRuntimeApi<Block>
		+ StorageProvider<Block, BE>
		+ sc_client_api::BlockBackend<Block>
		+ HeaderBackend<Block>
		+ AuxStore
		+ HeaderMetadata<Block, Error = BlockChainError>
		+ Sync
		+ Send
		+ 'static,
	C: BlockchainEvents<Block>,
	C::Api: substrate_frame_rpc_system::AccountNonceApi<Block, AccountId, Index>,
	C::Api: pallet_mmr_rpc::MmrRuntimeApi<
		Block,
		<Block as sp_runtime::traits::Block>::Hash,
		BlockNumber,
	>,
	C::Api: pallet_transaction_payment_rpc::TransactionPaymentRuntimeApi<Block, Balance>,
	C::Api: RRSCApi<Block>,
	C::Api: BlockBuilder<Block>,
	C::Api: fp_rpc::ConvertTransactionRuntimeApi<Block>,
	C::Api: fp_rpc::EthereumRuntimeRPCApi<Block>,
	P: TransactionPool<Block = Block> + 'static,
	B: sc_client_api::Backend<Block> + Send + Sync + 'static,
	B::State: sc_client_api::backend::StateBackend<sp_runtime::traits::HashFor<Block>>,
	A: ChainApi<Block = Block> + 'static,
	CT: fp_rpc::ConvertTransaction<<Block as BlockT>::Extrinsic> + Send + Sync + 'static,
{
	use cessc_consensus_rrsc_rpc::{RRSCApiServer, RRSC};
	use cessc_sync_state_rpc::{SyncState, SyncStateApiServer};
	use fc_rpc::{
		Eth, EthApiServer, EthDevSigner, EthFilter, EthFilterApiServer, EthPubSub,
		EthPubSubApiServer, EthSigner, Net, NetApiServer, Web3, Web3ApiServer,
	};
	use pallet_mmr_rpc::{Mmr, MmrApiServer};
	use pallet_transaction_payment_rpc::{TransactionPayment, TransactionPaymentApiServer};
	use sc_finality_grandpa_rpc::{Grandpa, GrandpaApiServer};
	use sc_rpc::dev::{Dev, DevApiServer};
	use sc_rpc_spec_v2::chain_spec::{ChainSpec, ChainSpecApiServer};
	use substrate_frame_rpc_system::{System, SystemApiServer};
	use substrate_state_trie_migration_rpc::{StateMigration, StateMigrationApiServer};

	let mut io = RpcModule::new(());
	let FullDeps {
		client,
		pool,
		deny_unsafe,
		graph,
		converter,
		is_authority,
		enable_dev_signer,
		network,
		filter_pool,
		frontier_backend,
		backend,
		max_past_logs,
		fee_history_limit,
		fee_history_cache,
		overrides,
		block_data_cache,
		execute_gas_limit_multiplier,
	} = deps;

	io.merge(System::new(client.clone(), pool.clone(), deny_unsafe).into_rpc())?;
	// Making synchronous calls in light client freezes the browser currently,
	// more context: https://github.com/paritytech/substrate/pull/3480
	// These RPCs should use an asynchronous caller instead.
	// io.merge(Contracts::new(client.clone()).into_rpc())?;
	io.merge(Mmr::new(client.clone()).into_rpc())?;
	io.merge(TransactionPayment::new(client.clone()).into_rpc())?;

	let mut signers = Vec::new();
	if enable_dev_signer {
		signers.push(Box::new(EthDevSigner::new()) as Box<dyn EthSigner>);
	}

	io.merge(
		Eth::new(
			client.clone(),
			pool.clone(),
			graph,
			converter,
			network.clone(),
			vec![],
			overrides.clone(),
			frontier_backend.clone(),
			is_authority,
			block_data_cache.clone(),
			fee_history_cache,
			fee_history_limit,
			execute_gas_limit_multiplier,
		)
		.into_rpc(),
	)?;

	if let Some(filter_pool) = filter_pool {
		io.merge(
			EthFilter::new(
				client.clone(),
				frontier_backend,
				filter_pool,
				500_usize, // max stored filters
				max_past_logs,
				block_data_cache,
			)
			.into_rpc(),
		)?;
	}

	io.merge(
		EthPubSub::new(pool, client.clone(), network.clone(), subscription_task_executor, overrides)
			.into_rpc(),
	)?;

	io.merge(
		Net::new(
			client.clone(),
			network,
			// Whether to format the `peer_count` response as Hex (default) or not.
			true,
		)
		.into_rpc(),
	)?;

	io.merge(Web3::new(client.clone()).into_rpc())?;
	io.merge(StateMigration::new(client.clone(), backend, deny_unsafe).into_rpc())?;
	io.merge(Dev::new(client, deny_unsafe).into_rpc())?;

	Ok(io)
}
