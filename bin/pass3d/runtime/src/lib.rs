// Copyright 2019-2021 Parity Technologies (UK) Ltd.
// This file is part of Parity Bridges Common.

// Parity Bridges Common is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity Bridges Common is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity Bridges Common.  If not, see <http://www.gnu.org/licenses/>.

//! The Pass3d runtime. This can be compiled with `#[no_std]`, ready for Wasm.

#![cfg_attr(not(feature = "std"), no_std)]
// `construct_runtime!` does a lot of recursion and requires us to increase the limit to 256.
#![recursion_limit = "256"]
// Runtime-generated enums
#![allow(clippy::large_enum_variant)]
// From construct_runtime macro
#![allow(clippy::from_over_into)]

// Make the WASM binary available.
#[cfg(feature = "std")]
include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));

pub mod pass3dt_messages;
pub mod xcm_config;

use crate::pass3dt_messages::{ToPass3dtMessagePayload, WithPass3dtMessageBridge};

use beefy_primitives::{crypto::AuthorityId as BeefyId, mmr::MmrLeafVersion, ValidatorSet};
use bp_runtime::{HeaderId, HeaderIdProvider};
use bridge_runtime_common::messages::{
	source::estimate_message_dispatch_and_delivery_fee, MessageBridge,
};
use pallet_grandpa::{
	fg_primitives, AuthorityId as GrandpaId, AuthorityList as GrandpaAuthorityList,
};
use pallet_mmr::primitives as mmr;
use pallet_transaction_payment::{FeeDetails, Multiplier, RuntimeDispatchInfo};
use sp_api::impl_runtime_apis;
use sp_core::{crypto::KeyTypeId, OpaqueMetadata};
use sp_mmr_primitives::{
	DataOrHash, EncodableOpaqueLeaf, Error as MmrError, LeafDataProvider, Proof as MmrProof,
};
use sp_runtime::{
	create_runtime_str, generic, impl_opaque_keys,
	traits::{AccountIdLookup, Block as BlockT, Keccak256, NumberFor, OpaqueKeys},
	transaction_validity::{TransactionSource, TransactionValidity},
	ApplyExtrinsicResult, FixedPointNumber, FixedU128, Perquintill,
};
use sp_std::prelude::*;
#[cfg(feature = "std")]
use sp_version::NativeVersion;
use sp_version::RuntimeVersion;

// A few exports that help ease life for downstream crates.
pub use frame_support::{
	construct_runtime, parameter_types,
	traits::{Currency, ExistenceRequirement, Imbalance, KeyOwnerProofSystem},
	weights::{constants::WEIGHT_PER_SECOND, DispatchClass, IdentityFee, RuntimeDbWeight, Weight},
	StorageValue,
};

pub use frame_system::Call as SystemCall;
pub use pallet_balances::Call as BalancesCall;
pub use pallet_bridge_grandpa::Call as BridgeGrandpaCall;
pub use pallet_bridge_messages::Call as MessagesCall;
pub use pallet_sudo::Call as SudoCall;
pub use pallet_timestamp::Call as TimestampCall;
pub use pallet_xcm::Call as XcmCall;

#[cfg(any(feature = "std", test))]
pub use sp_runtime::BuildStorage;
pub use sp_runtime::{Perbill, Permill};

/// An index to a block.
pub type BlockNumber = bp_pass3d::BlockNumber;

/// Alias to 512-bit hash when used in the context of a transaction signature on the chain.
pub type Signature = bp_pass3d::Signature;

/// Some way of identifying an account on the chain. We intentionally make it equivalent
/// to the public key of our transaction signing scheme.
pub type AccountId = bp_pass3d::AccountId;

/// The type for looking up accounts. We don't expect more than 4 billion of them, but you
/// never know...
pub type AccountIndex = u32;

/// Balance of an account.
pub type Balance = bp_pass3d::Balance;

/// Index of a transaction in the chain.
pub type Index = bp_pass3d::Index;

/// A hash of some data used by the chain.
pub type Hash = bp_pass3d::Hash;

/// Hashing algorithm used by the chain.
pub type Hashing = bp_pass3d::Hasher;

/// Opaque types. These are used by the CLI to instantiate machinery that don't need to know
/// the specifics of the runtime. They can then be made to be agnostic over specific formats
/// of data like extrinsics, allowing for them to continue syncing the network through upgrades
/// to even the core data structures.
pub mod opaque {
	use super::*;

	pub use sp_runtime::OpaqueExtrinsic as UncheckedExtrinsic;

	/// Opaque block header type.
	pub type Header = generic::Header<BlockNumber, Hashing>;
	/// Opaque block type.
	pub type Block = generic::Block<Header, UncheckedExtrinsic>;
	/// Opaque block identifier type.
	pub type BlockId = generic::BlockId<Block>;
}

impl_opaque_keys! {
	pub struct SessionKeys {
		pub babe: Babe,
		pub grandpa: Grandpa,
		pub beefy: Beefy,
		// pub para_validator: Initializer,
		// pub para_assignment: SessionInfo,
		pub authority_discovery: AuthorityDiscovery,
	}
}

/// This runtime version.
pub const VERSION: RuntimeVersion = RuntimeVersion {
	spec_name: create_runtime_str!("pass3d-runtime"),
	impl_name: create_runtime_str!("pass3d-runtime"),
	authoring_version: 1,
	spec_version: 1,
	impl_version: 1,
	apis: RUNTIME_API_VERSIONS,
	transaction_version: 1,
	state_version: 1,
};

/// The version information used to identify this runtime when compiled natively.
#[cfg(feature = "std")]
pub fn native_version() -> NativeVersion {
	NativeVersion { runtime_version: VERSION, can_author_with: Default::default() }
}

parameter_types! {
	pub const BlockHashCount: BlockNumber = 250;
	pub const Version: RuntimeVersion = VERSION;
	pub const DbWeight: RuntimeDbWeight = RuntimeDbWeight {
		read: 60_000_000, // ~0.06 ms = ~60 µs
		write: 200_000_000, // ~0.2 ms = 200 µs
	};
	pub const SS58Prefix: u8 = 48;
}

impl frame_system::Config for Runtime {
	/// The basic call filter to use in dispatchable.
	type BaseCallFilter = frame_support::traits::Everything;
	/// The identifier used to distinguish between accounts.
	type AccountId = AccountId;
	/// The aggregated dispatch type that is available for extrinsics.
	type Call = Call;
	/// The lookup mechanism to get account ID from whatever is passed in dispatchers.
	type Lookup = AccountIdLookup<AccountId, ()>;
	/// The index type for storing how many extrinsics an account has signed.
	type Index = Index;
	/// The index type for blocks.
	type BlockNumber = BlockNumber;
	/// The type for hashing blocks and tries.
	type Hash = Hash;
	/// The hashing algorithm used.
	type Hashing = Hashing;
	/// The header type.
	type Header = generic::Header<BlockNumber, Hashing>;
	/// The ubiquitous event type.
	type Event = Event;
	/// The ubiquitous origin type.
	type Origin = Origin;
	/// Maximum number of block number to block hash mappings to keep (oldest pruned first).
	type BlockHashCount = BlockHashCount;
	/// Version of the runtime.
	type Version = Version;
	/// Provides information about the pallet setup in the runtime.
	type PalletInfo = PalletInfo;
	/// What to do if a new account is created.
	type OnNewAccount = ();
	/// What to do if an account is fully reaped from the system.
	type OnKilledAccount = ();
	/// The data to be stored in an account.
	type AccountData = pallet_balances::AccountData<Balance>;
	// TODO: update me (https://github.com/paritytech/parity-bridges-common/issues/78)
	/// Weight information for the extrinsics of this pallet.
	type SystemWeightInfo = ();
	/// Block and extrinsics weights: base values and limits.
	type BlockWeights = bp_pass3d::BlockWeights;
	/// The maximum length of a block (in bytes).
	type BlockLength = bp_pass3d::BlockLength;
	/// The weight of database operations that the runtime can invoke.
	type DbWeight = DbWeight;
	/// The designated SS58 prefix of this chain.
	type SS58Prefix = SS58Prefix;
	/// The set code logic, just the default since we're not a parachain.
	type OnSetCode = ();
	type MaxConsumers = frame_support::traits::ConstU32<16>;
}

/// The BABE epoch configuration at genesis.
pub const BABE_GENESIS_EPOCH_CONFIG: sp_consensus_babe::BabeEpochConfiguration =
	sp_consensus_babe::BabeEpochConfiguration {
		c: bp_pass3d::time_units::PRIMARY_PROBABILITY,
		allowed_slots: sp_consensus_babe::AllowedSlots::PrimaryAndSecondaryVRFSlots,
	};

parameter_types! {
	pub const EpochDuration: u64 = bp_pass3d::EPOCH_DURATION_IN_SLOTS as u64;
	pub const ExpectedBlockTime: bp_pass3d::Moment = bp_pass3d::time_units::MILLISECS_PER_BLOCK;
	pub const MaxAuthorities: u32 = 10;
}

impl pallet_babe::Config for Runtime {
	type EpochDuration = EpochDuration;
	type ExpectedBlockTime = ExpectedBlockTime;
	type MaxAuthorities = MaxAuthorities;

	// session module is the trigger
	type EpochChangeTrigger = pallet_babe::ExternalTrigger;

	// equivocation related configuration - we don't expect any equivocations in our testnets
	type KeyOwnerProofSystem = ();
	type KeyOwnerProof = <Self::KeyOwnerProofSystem as KeyOwnerProofSystem<(
		KeyTypeId,
		pallet_babe::AuthorityId,
	)>>::Proof;
	type KeyOwnerIdentification = <Self::KeyOwnerProofSystem as KeyOwnerProofSystem<(
		KeyTypeId,
		pallet_babe::AuthorityId,
	)>>::IdentificationTuple;
	type HandleEquivocation = ();

	type DisabledValidators = ();
	type WeightInfo = ();
}

impl pallet_beefy::Config for Runtime {
	type BeefyId = BeefyId;
	type MaxAuthorities = MaxAuthorities;
	type OnNewValidatorSet = MmrLeaf;
}

impl pallet_grandpa::Config for Runtime {
	type Event = Event;
	type Call = Call;
	type MaxAuthorities = MaxAuthorities;
	type KeyOwnerProofSystem = ();
	type KeyOwnerProof =
		<Self::KeyOwnerProofSystem as KeyOwnerProofSystem<(KeyTypeId, GrandpaId)>>::Proof;
	type KeyOwnerIdentification = <Self::KeyOwnerProofSystem as KeyOwnerProofSystem<(
		KeyTypeId,
		GrandpaId,
	)>>::IdentificationTuple;
	type HandleEquivocation = ();
	// TODO: update me (https://github.com/paritytech/parity-bridges-common/issues/78)
	type WeightInfo = ();
}

type MmrHash = <Keccak256 as sp_runtime::traits::Hash>::Output;
type MmrHashing = <Runtime as pallet_mmr::Config>::Hashing;

impl pallet_mmr::Config for Runtime {
	const INDEXING_PREFIX: &'static [u8] = b"mmr";
	type Hashing = Keccak256;
	type Hash = <Keccak256 as sp_runtime::traits::Hash>::Output;
	type OnNewRoot = pallet_beefy_mmr::DepositBeefyDigest<Runtime>;
	type WeightInfo = ();
	type LeafData = pallet_beefy_mmr::Pallet<Runtime>;
}

parameter_types! {
	/// Version of the produced MMR leaf.
	///
	/// The version consists of two parts;
	/// - `major` (3 bits)
	/// - `minor` (5 bits)
	///
	/// `major` should be updated only if decoding the previous MMR Leaf format from the payload
	/// is not possible (i.e. backward incompatible change).
	/// `minor` should be updated if fields are added to the previous MMR Leaf, which given SCALE
	/// encoding does not prevent old leafs from being decoded.
	///
	/// Hence we expect `major` to be changed really rarely (think never).
	/// See [`MmrLeafVersion`] type documentation for more details.
	pub LeafVersion: MmrLeafVersion = MmrLeafVersion::new(0, 0);
}

pub struct BeefyDummyDataProvider;

impl beefy_primitives::mmr::BeefyDataProvider<()> for BeefyDummyDataProvider {
	fn extra_data() {}
}

impl pallet_beefy_mmr::Config for Runtime {
	type LeafVersion = LeafVersion;
	type BeefyAuthorityToMerkleLeaf = pallet_beefy_mmr::BeefyEcdsaToEthereum;
	type LeafExtra = ();
	type BeefyDataProvider = BeefyDummyDataProvider;
}

parameter_types! {
	pub const MinimumPeriod: u64 = bp_pass3d::SLOT_DURATION / 2;
}

impl pallet_timestamp::Config for Runtime {
	/// A timestamp: milliseconds since the UNIX epoch.
	type Moment = bp_pass3d::Moment;
	type OnTimestampSet = Babe;
	type MinimumPeriod = MinimumPeriod;
	// TODO: update me (https://github.com/paritytech/parity-bridges-common/issues/78)
	type WeightInfo = ();
}

parameter_types! {
	pub const ExistentialDeposit: bp_pass3d::Balance = 500;
	// For weight estimation, we assume that the most locks on an individual account will be 50.
	// This number may need to be adjusted in the future if this assumption no longer holds true.
	pub const MaxLocks: u32 = 50;
	pub const MaxReserves: u32 = 50;
}

impl pallet_balances::Config for Runtime {
	/// The type for recording an account's balance.
	type Balance = Balance;
	/// The ubiquitous event type.
	type Event = Event;
	type DustRemoval = ();
	type ExistentialDeposit = ExistentialDeposit;
	type AccountStore = System;
	// TODO: update me (https://github.com/paritytech/parity-bridges-common/issues/78)
	type WeightInfo = ();
	type MaxLocks = MaxLocks;
	type MaxReserves = MaxReserves;
	type ReserveIdentifier = [u8; 8];
}

parameter_types! {
	pub const TransactionBaseFee: Balance = 0;
	pub const TransactionByteFee: Balance = 1;
	pub const OperationalFeeMultiplier: u8 = 5;
	// values for following parameters are copied from polkadot repo, but it is fine
	// not to sync them - we're not going to make Pass3d a full copy of one of Polkadot-like chains
	pub const TargetBlockFullness: Perquintill = Perquintill::from_percent(25);
	pub AdjustmentVariable: Multiplier = Multiplier::saturating_from_rational(3, 100_000);
	pub MinimumMultiplier: Multiplier = Multiplier::saturating_from_rational(1, 1_000_000u128);
}

impl pallet_transaction_payment::Config for Runtime {
	type OnChargeTransaction = pallet_transaction_payment::CurrencyAdapter<Balances, ()>;
	type OperationalFeeMultiplier = OperationalFeeMultiplier;
	type WeightToFee = bp_pass3d::WeightToFee;
	type LengthToFee = bp_pass3d::WeightToFee;
	type FeeMultiplierUpdate = pallet_transaction_payment::TargetedFeeAdjustment<
		Runtime,
		TargetBlockFullness,
		AdjustmentVariable,
		MinimumMultiplier,
	>;
	type Event = Event;
}

impl pallet_sudo::Config for Runtime {
	type Event = Event;
	type Call = Call;
}

impl pallet_session::Config for Runtime {
	type Event = Event;
	type ValidatorId = <Self as frame_system::Config>::AccountId;
	type ValidatorIdOf = ();
	type ShouldEndSession = Babe;
	type NextSessionRotation = Babe;
	type SessionManager = pallet_shift_session_manager::Pallet<Runtime>;
	type SessionHandler = <SessionKeys as OpaqueKeys>::KeyTypeIdProviders;
	type Keys = SessionKeys;
	// TODO: update me (https://github.com/paritytech/parity-bridges-common/issues/78)
	type WeightInfo = ();
}

impl pallet_authority_discovery::Config for Runtime {
	type MaxAuthorities = MaxAuthorities;
}

impl pallet_bridge_relayers::Config for Runtime {
	type Event = Event;
	type Reward = Balance;
	type PaymentProcedure = bp_relayers::MintReward<pallet_balances::Pallet<Runtime>, AccountId>;
	type WeightInfo = ();
}

parameter_types! {
	/// This is a pretty unscientific cap.
	///
	/// Note that once this is hit the pallet will essentially throttle incoming requests down to one
	/// call per block.
	pub const MaxRequests: u32 = 50;

	/// Number of headers to keep.
	///
	/// Assuming the worst case of every header being finalized, we will keep headers at least for a
	/// week.
	pub const HeadersToKeep: u32 = 7 * bp_pass3d::DAYS as u32;
}

pub type Pass3dtGrandpaInstance = ();
impl pallet_bridge_grandpa::Config for Runtime {
	type BridgedChain = bp_pass3dt::Pass3dt;
	type MaxRequests = MaxRequests;
	type HeadersToKeep = HeadersToKeep;
	type WeightInfo = (); //pallet_bridge_grandpa::weights::Pass3dtWeight<Runtime>;
}

impl pallet_shift_session_manager::Config for Runtime {}

parameter_types! {
	pub const MaxMessagesToPruneAtOnce: bp_messages::MessageNonce = 8;
	pub const MaxUnrewardedRelayerEntriesAtInboundLane: bp_messages::MessageNonce =
		bp_pass3dt::MAX_UNREWARDED_RELAYERS_IN_CONFIRMATION_TX;
	pub const MaxUnconfirmedMessagesAtInboundLane: bp_messages::MessageNonce =
		bp_pass3dt::MAX_UNCONFIRMED_MESSAGES_IN_CONFIRMATION_TX;
	// `IdentityFee` is used by Pass3d => we may use weight directly
	pub const GetDeliveryConfirmationTransactionFee: Balance =
		bp_pass3d::MAX_SINGLE_MESSAGE_DELIVERY_CONFIRMATION_TX_WEIGHT as _;
	pub const RootAccountForPayments: Option<AccountId> = None;
	pub const BridgedChainId: bp_runtime::ChainId = bp_runtime::PASS3DT_CHAIN_ID;
}

/// Instance of the messages pallet used to relay messages to/from Pass3dt chain.
pub type WithPass3dtMessagesInstance = ();

impl pallet_bridge_messages::Config<WithPass3dtMessagesInstance> for Runtime {
	type Event = Event;
	type WeightInfo = (); //pallet_bridge_messages::weights::Pass3dtWeight<Runtime>;
	type Parameter = pass3dt_messages::Pass3dToPass3dtMessagesParameter;
	type MaxMessagesToPruneAtOnce = MaxMessagesToPruneAtOnce;
	type MaxUnrewardedRelayerEntriesAtInboundLane = MaxUnrewardedRelayerEntriesAtInboundLane;
	type MaxUnconfirmedMessagesAtInboundLane = MaxUnconfirmedMessagesAtInboundLane;

	type MaximalOutboundPayloadSize = crate::pass3dt_messages::ToPass3dtMaximalOutboundPayloadSize;
	type OutboundPayload = crate::pass3dt_messages::ToPass3dtMessagePayload;
	type OutboundMessageFee = Balance;

	type InboundPayload = crate::pass3dt_messages::FromPass3dtMessagePayload;
	type InboundMessageFee = bp_pass3dt::Balance;
	type InboundRelayer = bp_pass3dt::AccountId;

	type TargetHeaderChain = crate::pass3dt_messages::Pass3dt;
	type LaneMessageVerifier = crate::pass3dt_messages::ToPass3dtMessageVerifier;
	type MessageDeliveryAndDispatchPayment =
		pallet_bridge_relayers::MessageDeliveryAndDispatchPaymentAdapter<
			Runtime,
			WithPass3dtMessagesInstance,
			GetDeliveryConfirmationTransactionFee,
		>;
	type OnMessageAccepted = ();
	type OnDeliveryConfirmed = ();

	type SourceHeaderChain = crate::pass3dt_messages::Pass3dt;
	type MessageDispatch = crate::pass3dt_messages::FromPass3dtMessageDispatch;
	type BridgedChainId = BridgedChainId;
}

construct_runtime!(
	pub enum Runtime where
		Block = Block,
		NodeBlock = opaque::Block,
		UncheckedExtrinsic = UncheckedExtrinsic
	{
		System: frame_system::{Pallet, Call, Config, Storage, Event<T>},
		Sudo: pallet_sudo::{Pallet, Call, Config<T>, Storage, Event<T>},

		// Must be before session.
		Babe: pallet_babe::{Pallet, Call, Storage, Config, ValidateUnsigned},

		Timestamp: pallet_timestamp::{Pallet, Call, Storage, Inherent},
		Balances: pallet_balances::{Pallet, Call, Storage, Config<T>, Event<T>},
		TransactionPayment: pallet_transaction_payment::{Pallet, Storage, Event<T>},

		// Consensus support.
		AuthorityDiscovery: pallet_authority_discovery::{Pallet, Config},
		Session: pallet_session::{Pallet, Call, Storage, Event, Config<T>},
		Grandpa: pallet_grandpa::{Pallet, Call, Storage, Config, Event},
		ShiftSessionManager: pallet_shift_session_manager::{Pallet},

		// BEEFY Bridges support.
		Beefy: pallet_beefy::{Pallet, Storage, Config<T>},
		Mmr: pallet_mmr::{Pallet, Storage},
		MmrLeaf: pallet_beefy_mmr::{Pallet, Storage},

		// Pass3dt bridge modules.
		BridgeRelayers: pallet_bridge_relayers::{Pallet, Call, Storage, Event<T>},
		BridgePass3dtGrandpa: pallet_bridge_grandpa::{Pallet, Call, Storage},
		BridgePass3dtMessages: pallet_bridge_messages::{Pallet, Call, Storage, Event<T>, Config<T>},

		// Parachain modules.
		// ParachainsOrigin: polkadot_runtime_parachains::origin::{Pallet, Origin},
		// Configuration: polkadot_runtime_parachains::configuration::{Pallet, Call, Storage, Config<T>},
		// Shared: polkadot_runtime_parachains::shared::{Pallet, Call, Storage},
		// Inclusion: polkadot_runtime_parachains::inclusion::{Pallet, Call, Storage, Event<T>},
		// ParasInherent: polkadot_runtime_parachains::paras_inherent::{Pallet, Call, Storage, Inherent},
		// Scheduler: polkadot_runtime_parachains::scheduler::{Pallet, Storage},
		// Paras: polkadot_runtime_parachains::paras::{Pallet, Call, Storage, Event, Config},
		// Initializer: polkadot_runtime_parachains::initializer::{Pallet, Call, Storage},
		// Dmp: polkadot_runtime_parachains::dmp::{Pallet, Call, Storage},
		// Ump: polkadot_runtime_parachains::ump::{Pallet, Call, Storage, Event},
		// Hrmp: polkadot_runtime_parachains::hrmp::{Pallet, Call, Storage, Event<T>, Config},
		// SessionInfo: polkadot_runtime_parachains::session_info::{Pallet, Storage},

		// Parachain Onboarding Pallets
		// Registrar: polkadot_runtime_common::paras_registrar::{Pallet, Call, Storage, Event<T>},
		// Slots: polkadot_runtime_common::slots::{Pallet, Call, Storage, Event<T>},
		// ParasSudoWrapper: polkadot_runtime_common::paras_sudo_wrapper::{Pallet, Call},

		// Pallet for sending XCM.
		XcmPallet: pallet_xcm::{Pallet, Call, Storage, Event<T>, Origin, Config} = 99,
	}
);

/// The address format for describing accounts.
pub type Address = sp_runtime::MultiAddress<AccountId, ()>;
/// Block header type as expected by this runtime.
pub type Header = generic::Header<BlockNumber, Hashing>;
/// Block type as expected by this runtime.
pub type Block = generic::Block<Header, UncheckedExtrinsic>;
/// A Block signed with a Justification
pub type SignedBlock = generic::SignedBlock<Block>;
/// BlockId type as expected by this runtime.
pub type BlockId = generic::BlockId<Block>;
/// The SignedExtension to the basic transaction logic.
pub type SignedExtra = (
	frame_system::CheckNonZeroSender<Runtime>,
	frame_system::CheckSpecVersion<Runtime>,
	frame_system::CheckTxVersion<Runtime>,
	frame_system::CheckGenesis<Runtime>,
	frame_system::CheckEra<Runtime>,
	frame_system::CheckNonce<Runtime>,
	frame_system::CheckWeight<Runtime>,
	pallet_transaction_payment::ChargeTransactionPayment<Runtime>,
);
/// The payload being signed in transactions.
pub type SignedPayload = generic::SignedPayload<Call, SignedExtra>;
/// Unchecked extrinsic type as expected by this runtime.
pub type UncheckedExtrinsic = generic::UncheckedExtrinsic<Address, Call, Signature, SignedExtra>;
/// Extrinsic type that has already been checked.
pub type CheckedExtrinsic = generic::CheckedExtrinsic<AccountId, Call, SignedExtra>;
/// Executive: handles dispatch to the various modules.
pub type Executive = frame_executive::Executive<
	Runtime,
	Block,
	frame_system::ChainContext<Runtime>,
	Runtime,
	AllPalletsWithSystem,
>;

impl_runtime_apis! {
	impl sp_api::Core<Block> for Runtime {
		fn version() -> RuntimeVersion {
			VERSION
		}

		fn execute_block(block: Block) {
			Executive::execute_block(block);
		}

		fn initialize_block(header: &<Block as BlockT>::Header) {
			Executive::initialize_block(header)
		}
	}

	impl sp_api::Metadata<Block> for Runtime {
		fn metadata() -> OpaqueMetadata {
			OpaqueMetadata::new(Runtime::metadata().into())
		}
	}

	impl sp_block_builder::BlockBuilder<Block> for Runtime {
		fn apply_extrinsic(extrinsic: <Block as BlockT>::Extrinsic) -> ApplyExtrinsicResult {
			Executive::apply_extrinsic(extrinsic)
		}

		fn finalize_block() -> <Block as BlockT>::Header {
			Executive::finalize_block()
		}

		fn inherent_extrinsics(data: sp_inherents::InherentData) -> Vec<<Block as BlockT>::Extrinsic> {
			data.create_extrinsics()
		}

		fn check_inherents(
			block: Block,
			data: sp_inherents::InherentData,
		) -> sp_inherents::CheckInherentsResult {
			data.check_extrinsics(&block)
		}
	}

	impl frame_system_rpc_runtime_api::AccountNonceApi<Block, AccountId, Index> for Runtime {
		fn account_nonce(account: AccountId) -> Index {
			System::account_nonce(account)
		}
	}

	impl beefy_primitives::BeefyApi<Block> for Runtime {
		fn validator_set() -> Option<ValidatorSet<BeefyId>> {
			Beefy::validator_set()
		}
	}

	impl sp_mmr_primitives::MmrApi<Block, Hash> for Runtime {
		fn generate_proof(leaf_index: u64)
			-> Result<(EncodableOpaqueLeaf, MmrProof<Hash>), MmrError>
		{
			Mmr::generate_batch_proof(vec![leaf_index])
				.and_then(|(leaves, proof)| Ok((
					mmr::EncodableOpaqueLeaf::from_leaf(&leaves[0]),
					mmr::BatchProof::into_single_leaf_proof(proof)?
				)))
		}

		fn verify_proof(leaf: EncodableOpaqueLeaf, proof: MmrProof<Hash>)
			-> Result<(), MmrError>
		{
			pub type Leaf = <
				<Runtime as pallet_mmr::Config>::LeafData as LeafDataProvider
			>::LeafData;

			let leaf: Leaf = leaf
				.into_opaque_leaf()
				.try_decode()
				.ok_or(MmrError::Verify)?;
			Mmr::verify_leaves(vec![leaf], mmr::Proof::into_batch_proof(proof))
		}

		fn verify_proof_stateless(
			root: Hash,
			leaf: EncodableOpaqueLeaf,
			proof: MmrProof<Hash>
		) -> Result<(), MmrError> {
			let node = DataOrHash::Data(leaf.into_opaque_leaf());
			pallet_mmr::verify_leaves_proof::<MmrHashing, _>(
				root,
				vec![node],
				pallet_mmr::primitives::Proof::into_batch_proof(proof),
			)
		}

		fn mmr_root() -> Result<Hash, MmrError> {
			Ok(Mmr::mmr_root())
		}

		fn generate_batch_proof(leaf_indices: Vec<pallet_mmr::primitives::LeafIndex>)
			-> Result<(Vec<mmr::EncodableOpaqueLeaf>, mmr::BatchProof<MmrHash>), mmr::Error>
		{
			Mmr::generate_batch_proof(leaf_indices)
				.map(|(leaves, proof)| (leaves.into_iter().map(|leaf| mmr::EncodableOpaqueLeaf::from_leaf(&leaf)).collect(), proof))
		}

		fn verify_batch_proof(leaves: Vec<mmr::EncodableOpaqueLeaf>, proof: mmr::BatchProof<MmrHash>)
			-> Result<(), mmr::Error>
		{
			type Leaf = <
				<Runtime as pallet_mmr::Config>::LeafData as LeafDataProvider
			>::LeafData;
			let leaves = leaves.into_iter().map(|leaf|
				leaf.into_opaque_leaf()
				.try_decode()
				.ok_or(mmr::Error::Verify)).collect::<Result<Vec<Leaf>, mmr::Error>>()?;
			Mmr::verify_leaves(leaves, proof)
		}

		fn verify_batch_proof_stateless(
			root: MmrHash,
			leaves: Vec<mmr::EncodableOpaqueLeaf>,
			proof: mmr::BatchProof<MmrHash>
		) -> Result<(), mmr::Error> {
			let nodes = leaves.into_iter().map(|leaf|mmr::DataOrHash::Data(leaf.into_opaque_leaf())).collect();
			pallet_mmr::verify_leaves_proof::<MmrHashing, _>(root, nodes, proof)
		}
	}

	impl bp_pass3dt::Pass3dtFinalityApi<Block> for Runtime {
		fn best_finalized() -> Option<HeaderId<bp_pass3dt::Hash, bp_pass3dt::BlockNumber>> {
			BridgePass3dtGrandpa::best_finalized().map(|header| header.id())
		}
	}

	impl sp_transaction_pool::runtime_api::TaggedTransactionQueue<Block> for Runtime {
		fn validate_transaction(
			source: TransactionSource,
			tx: <Block as BlockT>::Extrinsic,
			block_hash: <Block as BlockT>::Hash,
		) -> TransactionValidity {
			Executive::validate_transaction(source, tx, block_hash)
		}
	}

	impl sp_offchain::OffchainWorkerApi<Block> for Runtime {
		fn offchain_worker(header: &<Block as BlockT>::Header) {
			Executive::offchain_worker(header)
		}
	}

	impl sp_consensus_babe::BabeApi<Block> for Runtime {
		fn configuration() -> sp_consensus_babe::BabeGenesisConfiguration {
			// The choice of `c` parameter (where `1 - c` represents the
			// probability of a slot being empty), is done in accordance to the
			// slot duration and expected target block time, for safely
			// resisting network delays of maximum two seconds.
			// <https://research.web3.foundation/en/latest/polkadot/BABE/Babe/#6-practical-results>
			sp_consensus_babe::BabeGenesisConfiguration {
				slot_duration: Babe::slot_duration(),
				epoch_length: EpochDuration::get(),
				c: BABE_GENESIS_EPOCH_CONFIG.c,
				genesis_authorities: Babe::authorities().to_vec(),
				randomness: Babe::randomness(),
				allowed_slots: BABE_GENESIS_EPOCH_CONFIG.allowed_slots,
			}
		}

		fn current_epoch_start() -> sp_consensus_babe::Slot {
			Babe::current_epoch_start()
		}

		fn current_epoch() -> sp_consensus_babe::Epoch {
			Babe::current_epoch()
		}

		fn next_epoch() -> sp_consensus_babe::Epoch {
			Babe::next_epoch()
		}

		fn generate_key_ownership_proof(
			_slot: sp_consensus_babe::Slot,
			_authority_id: sp_consensus_babe::AuthorityId,
		) -> Option<sp_consensus_babe::OpaqueKeyOwnershipProof> {
			None
		}

		fn submit_report_equivocation_unsigned_extrinsic(
			equivocation_proof: sp_consensus_babe::EquivocationProof<<Block as BlockT>::Header>,
			key_owner_proof: sp_consensus_babe::OpaqueKeyOwnershipProof,
		) -> Option<()> {
			let key_owner_proof = key_owner_proof.decode()?;

			Babe::submit_unsigned_equivocation_report(
				equivocation_proof,
				key_owner_proof,
			)
		}
	}

	// impl polkadot_primitives::runtime_api::ParachainHost<Block, Hash, BlockNumber> for Runtime {
	// 	fn validators() -> Vec<polkadot_primitives::v2::ValidatorId> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::validators::<Runtime>()
	// 	}
	//
	// 	fn validator_groups() -> (Vec<Vec<polkadot_primitives::v2::ValidatorIndex>>, polkadot_primitives::v2::GroupRotationInfo<BlockNumber>) {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::validator_groups::<Runtime>()
	// 	}
	//
	// 	fn availability_cores() -> Vec<polkadot_primitives::v2::CoreState<Hash, BlockNumber>> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::availability_cores::<Runtime>()
	// 	}
	//
	// 	fn persisted_validation_data(para_id: polkadot_primitives::v2::Id, assumption: polkadot_primitives::v2::OccupiedCoreAssumption)
	// 		-> Option<polkadot_primitives::v2::PersistedValidationData<Hash, BlockNumber>> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::persisted_validation_data::<Runtime>(para_id, assumption)
	// 	}
	//
	// 	fn assumed_validation_data(
	// 		para_id: polkadot_primitives::v2::Id,
	// 		expected_persisted_validation_data_hash: Hash,
	// 	) -> Option<(polkadot_primitives::v2::PersistedValidationData<Hash, BlockNumber>, polkadot_primitives::v2::ValidationCodeHash)> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::assumed_validation_data::<Runtime>(
	// 			para_id,
	// 			expected_persisted_validation_data_hash,
	// 		)
	// 	}
	//
	// 	fn check_validation_outputs(
	// 		para_id: polkadot_primitives::v2::Id,
	// 		outputs: polkadot_primitives::v2::CandidateCommitments,
	// 	) -> bool {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::check_validation_outputs::<Runtime>(para_id, outputs)
	// 	}
	//
	// 	fn session_index_for_child() -> polkadot_primitives::v2::SessionIndex {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::session_index_for_child::<Runtime>()
	// 	}
	//
	// 	fn validation_code(para_id: polkadot_primitives::v2::Id, assumption: polkadot_primitives::v2::OccupiedCoreAssumption)
	// 		-> Option<polkadot_primitives::v2::ValidationCode> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::validation_code::<Runtime>(para_id, assumption)
	// 	}
	//
	// 	fn candidate_pending_availability(para_id: polkadot_primitives::v2::Id) -> Option<polkadot_primitives::v2::CommittedCandidateReceipt<Hash>> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::candidate_pending_availability::<Runtime>(para_id)
	// 	}
	//
	// 	fn candidate_events() -> Vec<polkadot_primitives::v2::CandidateEvent<Hash>> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::candidate_events::<Runtime, _>(|ev| {
	// 			match ev {
	// 				Event::Inclusion(ev) => {
	// 					Some(ev)
	// 				}
	// 				_ => None,
	// 			}
	// 		})
	// 	}
	//
	// 	fn session_info(index: polkadot_primitives::v2::SessionIndex) -> Option<polkadot_primitives::v2::SessionInfo> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::session_info::<Runtime>(index)
	// 	}
	//
	// 	fn dmq_contents(recipient: polkadot_primitives::v2::Id) -> Vec<polkadot_primitives::v2::InboundDownwardMessage<BlockNumber>> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::dmq_contents::<Runtime>(recipient)
	// 	}
	//
	// 	fn inbound_hrmp_channels_contents(
	// 		recipient: polkadot_primitives::v2::Id
	// 	) -> BTreeMap<polkadot_primitives::v2::Id, Vec<polkadot_primitives::v2::InboundHrmpMessage<BlockNumber>>> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::inbound_hrmp_channels_contents::<Runtime>(recipient)
	// 	}
	//
	// 	fn validation_code_by_hash(hash: polkadot_primitives::v2::ValidationCodeHash) -> Option<polkadot_primitives::v2::ValidationCode> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::validation_code_by_hash::<Runtime>(hash)
	// 	}
	//
	// 	fn on_chain_votes() -> Option<polkadot_primitives::v2::ScrapedOnChainVotes<Hash>> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::on_chain_votes::<Runtime>()
	// 	}
	//
	// 	fn submit_pvf_check_statement(stmt: polkadot_primitives::v2::PvfCheckStatement, signature: polkadot_primitives::v2::ValidatorSignature) {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::submit_pvf_check_statement::<Runtime>(stmt, signature)
	// 	}
	//
	// 	fn pvfs_require_precheck() -> Vec<polkadot_primitives::v2::ValidationCodeHash> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::pvfs_require_precheck::<Runtime>()
	// 	}
	//
	// 	fn validation_code_hash(para_id: polkadot_primitives::v2::Id, assumption: polkadot_primitives::v2::OccupiedCoreAssumption)
	// 		-> Option<polkadot_primitives::v2::ValidationCodeHash>
	// 	{
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::validation_code_hash::<Runtime>(para_id, assumption)
	// 	}
	//
	// 	fn staging_get_disputes() -> Vec<(polkadot_primitives::v2::SessionIndex, polkadot_primitives::v2::CandidateHash, polkadot_primitives::v2::DisputeState<BlockNumber>)> {
	// 		unimplemented!()
	// 	}
	// }

	// impl sp_authority_discovery::AuthorityDiscoveryApi<Block> for Runtime {
	// 	fn authorities() -> Vec<AuthorityDiscoveryId> {
	// 		polkadot_runtime_parachains::runtime_api_impl::v2::relevant_authority_ids::<Runtime>()
	// 	}
	// }

	impl pallet_transaction_payment_rpc_runtime_api::TransactionPaymentApi<
		Block,
		Balance,
	> for Runtime {
		fn query_info(uxt: <Block as BlockT>::Extrinsic, len: u32) -> RuntimeDispatchInfo<Balance> {
			TransactionPayment::query_info(uxt, len)
		}
		fn query_fee_details(uxt: <Block as BlockT>::Extrinsic, len: u32) -> FeeDetails<Balance> {
			TransactionPayment::query_fee_details(uxt, len)
		}
	}

	impl sp_session::SessionKeys<Block> for Runtime {
		fn generate_session_keys(seed: Option<Vec<u8>>) -> Vec<u8> {
			SessionKeys::generate(seed)
		}

		fn decode_session_keys(
			encoded: Vec<u8>,
		) -> Option<Vec<(Vec<u8>, sp_core::crypto::KeyTypeId)>> {
			SessionKeys::decode_into_raw_public_keys(&encoded)
		}
	}

	impl fg_primitives::GrandpaApi<Block> for Runtime {
		fn current_set_id() -> fg_primitives::SetId {
			Grandpa::current_set_id()
		}

		fn grandpa_authorities() -> GrandpaAuthorityList {
			Grandpa::grandpa_authorities()
		}

		fn submit_report_equivocation_unsigned_extrinsic(
			equivocation_proof: fg_primitives::EquivocationProof<
				<Block as BlockT>::Hash,
				NumberFor<Block>,
			>,
			key_owner_proof: fg_primitives::OpaqueKeyOwnershipProof,
		) -> Option<()> {
			let key_owner_proof = key_owner_proof.decode()?;

			Grandpa::submit_unsigned_equivocation_report(
				equivocation_proof,
				key_owner_proof,
			)
		}

		fn generate_key_ownership_proof(
			_set_id: fg_primitives::SetId,
			_authority_id: GrandpaId,
		) -> Option<fg_primitives::OpaqueKeyOwnershipProof> {
			// NOTE: this is the only implementation possible since we've
			// defined our key owner proof type as a bottom type (i.e. a type
			// with no values).
			None
		}
	}

	impl bp_pass3dt::ToPass3dtOutboundLaneApi<Block, Balance, ToPass3dtMessagePayload> for Runtime {
		fn estimate_message_delivery_and_dispatch_fee(
			_lane_id: bp_messages::LaneId,
			payload: ToPass3dtMessagePayload,
			pass3dt_to_this_conversion_rate: Option<FixedU128>,
		) -> Option<Balance> {
			estimate_message_dispatch_and_delivery_fee::<WithPass3dtMessageBridge>(
				&payload,
				WithPass3dtMessageBridge::RELAYER_FEE_PERCENT,
				pass3dt_to_this_conversion_rate,
			).ok()
		}

		fn message_details(
			lane: bp_messages::LaneId,
			begin: bp_messages::MessageNonce,
			end: bp_messages::MessageNonce,
		) -> Vec<bp_messages::OutboundMessageDetails<Balance>> {
			bridge_runtime_common::messages_api::outbound_message_details::<
				Runtime,
				WithPass3dtMessagesInstance,
			>(lane, begin, end)
		}
	}

	impl bp_pass3dt::FromPass3dtInboundLaneApi<Block, bp_pass3dt::Balance> for Runtime {
		fn message_details(
			lane: bp_messages::LaneId,
			messages: Vec<(bp_messages::MessagePayload, bp_messages::OutboundMessageDetails<bp_pass3dt::Balance>)>,
		) -> Vec<bp_messages::InboundMessageDetails> {
			bridge_runtime_common::messages_api::inbound_message_details::<
				Runtime,
				WithPass3dtMessagesInstance,
			>(lane, messages)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn call_size() {
		const BRIDGES_PALLETS_MAX_CALL_SIZE: usize = 200;
		assert!(
			core::mem::size_of::<pallet_bridge_grandpa::Call<Runtime>>() <=
				BRIDGES_PALLETS_MAX_CALL_SIZE
		);
		assert!(
			core::mem::size_of::<pallet_bridge_messages::Call<Runtime>>() <=
				BRIDGES_PALLETS_MAX_CALL_SIZE
		);
		// Largest inner Call is `pallet_session::Call` with a size of 224 bytes. This size is a
		// result of large `SessionKeys` struct.
		// Total size of Pass3d runtime Call is 232.
		const MAX_CALL_SIZE: usize = 232;
		assert!(core::mem::size_of::<Call>() <= MAX_CALL_SIZE);
	}
}
