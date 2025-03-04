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

//! The Rialto parachain runtime. This can be compiled with `#[no_std]`, ready for Wasm.
//!
//! Originally a copy of runtime from https://github.com/substrate-developer-hub/substrate-parachain-template.

#![cfg_attr(not(feature = "std"), no_std)]
// `construct_runtime!` does a lot of recursion and requires us to increase the limit to 256.
#![recursion_limit = "256"]

// Make the WASM binary available.
#[cfg(feature = "std")]
include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));

use crate::millau_messages::{
	ToMillauMessagePayload, WithMillauMessageBridge, DEFAULT_XCM_LANE_TO_MILLAU,
};

use bridge_runtime_common::messages::{
	source::{estimate_message_dispatch_and_delivery_fee, XcmBridge, XcmBridgeAdapter},
	MessageBridge,
};
use cumulus_pallet_parachain_system::AnyRelayNumber;
use sp_api::impl_runtime_apis;
use sp_core::{crypto::KeyTypeId, OpaqueMetadata};
use sp_runtime::{
	create_runtime_str, generic, impl_opaque_keys,
	traits::{AccountIdLookup, Block as BlockT},
	transaction_validity::{TransactionSource, TransactionValidity},
	ApplyExtrinsicResult, FixedU128,
};

use sp_std::prelude::*;
#[cfg(feature = "std")]
use sp_version::NativeVersion;
use sp_version::RuntimeVersion;

// A few exports that help ease life for downstream crates.
use bp_runtime::{HeaderId, HeaderIdProvider};
pub use frame_support::{
	construct_runtime, match_types, parameter_types,
	traits::{Everything, IsInVec, Nothing, Randomness},
	weights::{
		constants::{BlockExecutionWeight, ExtrinsicBaseWeight, RocksDbWeight, WEIGHT_PER_SECOND},
		DispatchClass, IdentityFee, Weight,
	},
	StorageValue,
};
pub use frame_system::{Call as SystemCall, EnsureRoot};
pub use pallet_balances::Call as BalancesCall;
pub use pallet_sudo::Call as SudoCall;
pub use pallet_timestamp::Call as TimestampCall;
pub use sp_consensus_aura::sr25519::AuthorityId as AuraId;
#[cfg(any(feature = "std", test))]
pub use sp_runtime::BuildStorage;
pub use sp_runtime::{MultiAddress, Perbill, Permill};

pub use bp_rialto_parachain::{
	AccountId, Balance, BlockLength, BlockNumber, BlockWeights, Hash, Hasher as Hashing, Header,
	Index, Signature, MAXIMUM_BLOCK_WEIGHT,
};

pub use pallet_bridge_grandpa::Call as BridgeGrandpaCall;
pub use pallet_bridge_messages::Call as MessagesCall;
pub use pallet_xcm::Call as XcmCall;

// Polkadot & XCM imports
use bridge_runtime_common::CustomNetworkId;
use pallet_xcm::XcmPassthrough;
use polkadot_parachain::primitives::Sibling;
use xcm::latest::prelude::*;
use xcm_builder::{
	AccountId32Aliases, AllowTopLevelPaidExecutionFrom, AllowUnpaidExecutionFrom, CurrencyAdapter,
	EnsureXcmOrigin, FixedWeightBounds, IsConcrete, NativeAsset, ParentAsSuperuser, ParentIsPreset,
	RelayChainAsNative, SiblingParachainAsNative, SiblingParachainConvertsVia,
	SignedAccountId32AsNative, SignedToAccountId32, SovereignSignedViaLocation, TakeWeightCredit,
	UsingComponents,
};
use xcm_executor::{Config, XcmExecutor};

pub mod millau_messages;

/// The address format for describing accounts.
pub type Address = MultiAddress<AccountId, ()>;
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

impl_opaque_keys! {
	pub struct SessionKeys {
		pub aura: Aura,
	}
}

/// This runtime version.
#[sp_version::runtime_version]
pub const VERSION: RuntimeVersion = RuntimeVersion {
	spec_name: create_runtime_str!("template-parachain"),
	impl_name: create_runtime_str!("template-parachain"),
	authoring_version: 1,
	spec_version: 1,
	impl_version: 0,
	apis: RUNTIME_API_VERSIONS,
	transaction_version: 1,
	state_version: 0,
};

/// This determines the average expected block time that we are targeting.
/// Blocks will be produced at a minimum duration defined by `SLOT_DURATION`.
/// `SLOT_DURATION` is picked up by `pallet_timestamp` which is in turn picked
/// up by `pallet_aura` to implement `fn slot_duration()`.
///
/// Change this to adjust the block time.
pub const MILLISECS_PER_BLOCK: u64 = 12000;

pub const SLOT_DURATION: u64 = MILLISECS_PER_BLOCK;

pub const EPOCH_DURATION_IN_BLOCKS: u32 = 10 * MINUTES;

// Time is measured by number of blocks.
pub const MINUTES: BlockNumber = 60_000 / (MILLISECS_PER_BLOCK as BlockNumber);
pub const HOURS: BlockNumber = MINUTES * 60;
pub const DAYS: BlockNumber = HOURS * 24;

// Unit = the base number of indivisible units for balances
pub const UNIT: Balance = 1_000_000_000_000;
pub const MILLIUNIT: Balance = 1_000_000_000;
pub const MICROUNIT: Balance = 1_000_000;

// 1 in 4 blocks (on average, not counting collisions) will be primary babe blocks.
pub const PRIMARY_PROBABILITY: (u64, u64) = (1, 4);

/// The version information used to identify this runtime when compiled natively.
#[cfg(feature = "std")]
pub fn native_version() -> NativeVersion {
	NativeVersion { runtime_version: VERSION, can_author_with: Default::default() }
}

parameter_types! {
	pub const BlockHashCount: BlockNumber = 250;
	pub const Version: RuntimeVersion = VERSION;
	pub const SS58Prefix: u8 = 48;
}

// Configure FRAME pallets to include in runtime.

impl frame_system::Config for Runtime {
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
	/// Runtime version.
	type Version = Version;
	/// Converts a module to an index of this module in the runtime.
	type PalletInfo = PalletInfo;
	type AccountData = pallet_balances::AccountData<Balance>;
	/// What to do if a new account is created.
	type OnNewAccount = ();
	/// What to do if an account is fully reaped from the system.
	type OnKilledAccount = ();
	/// The weight of database operations that the runtime can invoke.
	type DbWeight = ();
	/// The basic call filter to use in dispatchable.
	type BaseCallFilter = Everything;
	/// Weight information for the extrinsics of this pallet.
	type SystemWeightInfo = ();
	/// Block & extrinsics weights: base values and limits.
	type BlockWeights = BlockWeights;
	/// The maximum length of a block (in bytes).
	type BlockLength = BlockLength;
	/// This is used as an identifier of the chain. 42 is the generic substrate prefix.
	type SS58Prefix = SS58Prefix;
	/// The action to take on a Runtime Upgrade
	type OnSetCode = cumulus_pallet_parachain_system::ParachainSetCode<Self>;
	type MaxConsumers = frame_support::traits::ConstU32<16>;
}

parameter_types! {
	pub const MinimumPeriod: u64 = SLOT_DURATION / 2;
}

impl pallet_timestamp::Config for Runtime {
	/// A timestamp: milliseconds since the Unix epoch.
	type Moment = u64;
	type OnTimestampSet = ();
	type MinimumPeriod = MinimumPeriod;
	type WeightInfo = ();
}

parameter_types! {
	pub const ExistentialDeposit: u128 = MILLIUNIT;
	pub const TransferFee: u128 = MILLIUNIT;
	pub const CreationFee: u128 = MILLIUNIT;
	pub const TransactionByteFee: u128 = MICROUNIT;
	pub const OperationalFeeMultiplier: u8 = 5;
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
	type WeightInfo = pallet_balances::weights::SubstrateWeight<Runtime>;
	type MaxLocks = MaxLocks;
	type MaxReserves = MaxReserves;
	type ReserveIdentifier = [u8; 8];
}

impl pallet_transaction_payment::Config for Runtime {
	type OnChargeTransaction = pallet_transaction_payment::CurrencyAdapter<Balances, ()>;
	type OperationalFeeMultiplier = OperationalFeeMultiplier;
	type WeightToFee = IdentityFee<Balance>;
	type LengthToFee = IdentityFee<Balance>;
	type FeeMultiplierUpdate = ();
	type Event = Event;
}

impl pallet_sudo::Config for Runtime {
	type Call = Call;
	type Event = Event;
}

parameter_types! {
	pub const ReservedXcmpWeight: Weight = MAXIMUM_BLOCK_WEIGHT / 4;
	pub const ReservedDmpWeight: Weight = MAXIMUM_BLOCK_WEIGHT / 4;
}

impl cumulus_pallet_parachain_system::Config for Runtime {
	type Event = Event;
	type OnSystemEvent = ();
	type SelfParaId = parachain_info::Pallet<Runtime>;
	type OutboundXcmpMessageSource = XcmpQueue;
	type DmpMessageHandler = DmpQueue;
	type ReservedDmpWeight = ReservedDmpWeight;
	type XcmpMessageHandler = XcmpQueue;
	type ReservedXcmpWeight = ReservedXcmpWeight;
	type CheckAssociatedRelayNumber = AnyRelayNumber;
}

impl parachain_info::Config for Runtime {}

impl cumulus_pallet_aura_ext::Config for Runtime {}

impl pallet_randomness_collective_flip::Config for Runtime {}

parameter_types! {
	pub const RelayLocation: MultiLocation = MultiLocation::parent();
	pub const RelayNetwork: NetworkId = CustomNetworkId::Rialto.as_network_id();
	pub RelayOrigin: Origin = cumulus_pallet_xcm::Origin::Relay.into();
	pub UniversalLocation: InteriorMultiLocation = X1(Parachain(ParachainInfo::parachain_id().into()));
	/// The Millau network ID.
	pub const MillauNetwork: NetworkId = CustomNetworkId::Millau.as_network_id();
	/// The RialtoParachain network ID.
	pub const ThisNetwork: NetworkId = CustomNetworkId::RialtoParachain.as_network_id();
}

/// Type for specifying how a `MultiLocation` can be converted into an `AccountId`. This is used
/// when determining ownership of accounts for asset transacting and when attempting to use XCM
/// `Transact` in order to determine the dispatch Origin.
pub type LocationToAccountId = (
	// The parent (Relay-chain) origin converts to the default `AccountId`.
	ParentIsPreset<AccountId>,
	// Sibling parachain origins convert to AccountId via the `ParaId::into`.
	SiblingParachainConvertsVia<Sibling, AccountId>,
	// Straight up local `AccountId32` origins just alias directly to `AccountId`.
	AccountId32Aliases<RelayNetwork, AccountId>,
);

/// Means for transacting assets on this chain.
pub type LocalAssetTransactor = CurrencyAdapter<
	// Use this currency:
	Balances,
	// Use this currency when it is a fungible asset matching the given location or name:
	IsConcrete<RelayLocation>,
	// Do a simple punn to convert an AccountId32 MultiLocation into a native chain account ID:
	LocationToAccountId,
	// Our chain's account ID type (we can't get away without mentioning it explicitly):
	AccountId,
	// We don't track any teleports.
	(),
>;

/// This is the type we use to convert an (incoming) XCM origin into a local `Origin` instance,
/// ready for dispatching a transaction with XCM `Transact`. There is an `OriginKind` which can
/// biases the kind of local `Origin` it will become.
pub type XcmOriginToTransactDispatchOrigin = (
	// Sovereign account converter; this attempts to derive an `AccountId` from the origin location
	// using `LocationToAccountId` and then turn that into the usual `Signed` origin. Useful for
	// foreign chains who want to have a local sovereign account on this chain which they control.
	SovereignSignedViaLocation<LocationToAccountId, Origin>,
	// Native converter for Relay-chain (Parent) location; will converts to a `Relay` origin when
	// recognised.
	RelayChainAsNative<RelayOrigin, Origin>,
	// Native converter for sibling Parachains; will convert to a `SiblingPara` origin when
	// recognised.
	SiblingParachainAsNative<cumulus_pallet_xcm::Origin, Origin>,
	// Superuser converter for the Relay-chain (Parent) location. This will allow it to issue a
	// transaction from the Root origin.
	ParentAsSuperuser<Origin>,
	// Native signed account converter; this just converts an `AccountId32` origin into a normal
	// `Origin::Signed` origin of the same 32-byte value.
	SignedAccountId32AsNative<RelayNetwork, Origin>,
	// Xcm origins can be represented natively under the Xcm pallet's Xcm origin.
	XcmPassthrough<Origin>,
);

// TODO: until https://github.com/paritytech/parity-bridges-common/issues/1417 is fixed (in either way),
// the following constant must match the similar constant in the Millau runtime.

/// One XCM operation is `1_000_000_000` weight - almost certainly a conservative estimate.
pub const BASE_XCM_WEIGHT: Weight = 1_000_000_000;

parameter_types! {
	pub UnitWeightCost: Weight = BASE_XCM_WEIGHT;
	// One UNIT buys 1 second of weight.
	pub const WeightPrice: (MultiLocation, u128) = (MultiLocation::parent(), UNIT);
	pub const MaxInstructions: u32 = 100;
	pub const MaxAuthorities: u32 = 100_000;
	pub MaxAssetsIntoHolding: u32 = 64;
}

match_types! {
	pub type ParentOrParentsUnitPlurality: impl Contains<MultiLocation> = {
		MultiLocation { parents: 1, interior: Here } |
		MultiLocation { parents: 1, interior: X1(Plurality { id: BodyId::Unit, .. }) }
	};
}

pub type Barrier = (
	TakeWeightCredit,
	AllowTopLevelPaidExecutionFrom<Everything>,
	AllowUnpaidExecutionFrom<ParentOrParentsUnitPlurality>,
	// ^^^ Parent & its unit plurality gets free execution
);

/// XCM weigher type.
pub type XcmWeigher = FixedWeightBounds<UnitWeightCost, Call, MaxInstructions>;

pub struct XcmConfig;
impl Config for XcmConfig {
	type Call = Call;
	type XcmSender = XcmRouter;
	type AssetTransactor = LocalAssetTransactor;
	type OriginConverter = XcmOriginToTransactDispatchOrigin;
	type IsReserve = NativeAsset;
	type IsTeleporter = NativeAsset; // <- should be enough to allow teleportation of UNIT
	type UniversalLocation = UniversalLocation;
	type Barrier = Barrier;
	type Weigher = XcmWeigher;
	type Trader = UsingComponents<IdentityFee<Balance>, RelayLocation, AccountId, Balances, ()>;
	type ResponseHandler = PolkadotXcm;
	type AssetTrap = PolkadotXcm;
	type AssetClaims = PolkadotXcm;
	type SubscriptionService = PolkadotXcm;
	type PalletInstancesInfo = ();
	type MaxAssetsIntoHolding = MaxAssetsIntoHolding;
	type AssetLocker = ();
	type AssetExchanger = ();
	type FeeManager = ();
	type MessageExporter = ();
	type UniversalAliases = Nothing;
	type CallDispatcher = Call;
}

/// No local origins on this chain are allowed to dispatch XCM sends/executions.
pub type LocalOriginToLocation = SignedToAccountId32<Origin, AccountId, RelayNetwork>;

/// The means for routing XCM messages which are not for local execution into the right message
/// queues.
pub type XcmRouter = (
	// Bridge is used to communicate with other relay chain (Millau).
	XcmBridgeAdapter<ToMillauBridge>,
);

/// With-Millau bridge.
pub struct ToMillauBridge;

impl XcmBridge for ToMillauBridge {
	type MessageBridge = WithMillauMessageBridge;
	type MessageSender = pallet_bridge_messages::Pallet<Runtime, WithMillauMessagesInstance>;

	fn universal_location() -> InteriorMultiLocation {
		UniversalLocation::get()
	}

	fn verify_destination(dest: &MultiLocation) -> bool {
		matches!(*dest, MultiLocation { parents: 1, interior: X1(GlobalConsensus(r)) } if r == MillauNetwork::get())
	}

	fn build_destination() -> MultiLocation {
		let dest: InteriorMultiLocation = MillauNetwork::get().into();
		let here = UniversalLocation::get();
		dest.relative_to(&here)
	}

	fn xcm_lane() -> bp_messages::LaneId {
		DEFAULT_XCM_LANE_TO_MILLAU
	}
}

impl pallet_xcm::Config for Runtime {
	type Event = Event;
	type SendXcmOrigin = EnsureXcmOrigin<Origin, LocalOriginToLocation>;
	type XcmRouter = XcmRouter;
	type ExecuteXcmOrigin = EnsureXcmOrigin<Origin, LocalOriginToLocation>;
	type XcmExecuteFilter = Everything;
	type XcmExecutor = XcmExecutor<XcmConfig>;
	type XcmTeleportFilter = Everything;
	type XcmReserveTransferFilter = Everything;
	type Weigher = XcmWeigher;
	type Origin = Origin;
	type Call = Call;
	const VERSION_DISCOVERY_QUEUE_SIZE: u32 = 100;
	type AdvertisedXcmVersion = pallet_xcm::CurrentXcmVersion;
	type Currency = Balances;
	type CurrencyMatcher = ();
	type TrustedLockers = ();
	type SovereignAccountOf = ();
	type MaxLockers = frame_support::traits::ConstU32<8>;
	type UniversalLocation = UniversalLocation;
}

impl cumulus_pallet_xcm::Config for Runtime {
	type Event = Event;
	type XcmExecutor = XcmExecutor<XcmConfig>;
}

impl cumulus_pallet_xcmp_queue::Config for Runtime {
	type Event = Event;
	type XcmExecutor = XcmExecutor<XcmConfig>;
	type ChannelInfo = ParachainSystem;
	type VersionWrapper = ();
	type ExecuteOverweightOrigin = EnsureRoot<AccountId>;
	type ControllerOrigin = EnsureRoot<AccountId>;
	type ControllerOriginConverter = XcmOriginToTransactDispatchOrigin;
	type WeightInfo = ();
	type PriceForSiblingDelivery = ();
}

impl cumulus_pallet_dmp_queue::Config for Runtime {
	type Event = Event;
	type XcmExecutor = XcmExecutor<XcmConfig>;
	type ExecuteOverweightOrigin = frame_system::EnsureRoot<AccountId>;
}

impl pallet_aura::Config for Runtime {
	type AuthorityId = AuraId;
	type DisabledValidators = ();
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
	pub const HeadersToKeep: u32 = 7 * bp_millau::DAYS as u32;
}

pub type MillauGrandpaInstance = ();
impl pallet_bridge_grandpa::Config for Runtime {
	type BridgedChain = bp_millau::Millau;
	type MaxRequests = MaxRequests;
	type HeadersToKeep = HeadersToKeep;
	type WeightInfo = pallet_bridge_grandpa::weights::BridgeWeight<Runtime>;
}

parameter_types! {
	pub const MaxMessagesToPruneAtOnce: bp_messages::MessageNonce = 8;
	pub const MaxUnrewardedRelayerEntriesAtInboundLane: bp_messages::MessageNonce =
		bp_millau::MAX_UNREWARDED_RELAYERS_IN_CONFIRMATION_TX;
	pub const MaxUnconfirmedMessagesAtInboundLane: bp_messages::MessageNonce =
		bp_millau::MAX_UNCONFIRMED_MESSAGES_IN_CONFIRMATION_TX;
	// `IdentityFee` is used by Rialto => we may use weight directly
	pub const GetDeliveryConfirmationTransactionFee: Balance =
		bp_rialto_parachain::MAX_SINGLE_MESSAGE_DELIVERY_CONFIRMATION_TX_WEIGHT as _;
	pub const RootAccountForPayments: Option<AccountId> = None;
	pub const BridgedChainId: bp_runtime::ChainId = bp_runtime::MILLAU_CHAIN_ID;
}

/// Instance of the messages pallet used to relay messages to/from Millau chain.
pub type WithMillauMessagesInstance = ();

impl pallet_bridge_messages::Config<WithMillauMessagesInstance> for Runtime {
	type Event = Event;
	type WeightInfo = pallet_bridge_messages::weights::BridgeWeight<Runtime>;
	type Parameter = millau_messages::RialtoParachainToMillauMessagesParameter;
	type MaxMessagesToPruneAtOnce = MaxMessagesToPruneAtOnce;
	type MaxUnrewardedRelayerEntriesAtInboundLane = MaxUnrewardedRelayerEntriesAtInboundLane;
	type MaxUnconfirmedMessagesAtInboundLane = MaxUnconfirmedMessagesAtInboundLane;

	type MaximalOutboundPayloadSize = crate::millau_messages::ToMillauMaximalOutboundPayloadSize;
	type OutboundPayload = crate::millau_messages::ToMillauMessagePayload;
	type OutboundMessageFee = Balance;

	type InboundPayload = crate::millau_messages::FromMillauMessagePayload;
	type InboundMessageFee = bp_millau::Balance;
	type InboundRelayer = bp_millau::AccountId;

	type TargetHeaderChain = crate::millau_messages::Millau;
	type LaneMessageVerifier = crate::millau_messages::ToMillauMessageVerifier;
	type MessageDeliveryAndDispatchPayment =
		pallet_bridge_relayers::MessageDeliveryAndDispatchPaymentAdapter<
			Runtime,
			WithMillauMessagesInstance,
			GetDeliveryConfirmationTransactionFee,
		>;
	type OnMessageAccepted = ();
	type OnDeliveryConfirmed = ();

	type SourceHeaderChain = crate::millau_messages::Millau;
	type MessageDispatch = crate::millau_messages::FromMillauMessageDispatch;
	type BridgedChainId = BridgedChainId;
}

// Create the runtime by composing the FRAME pallets that were previously configured.
construct_runtime!(
	pub enum Runtime where
		Block = Block,
		NodeBlock = generic::Block<Header, sp_runtime::OpaqueExtrinsic>,
		UncheckedExtrinsic = UncheckedExtrinsic,
	{
		System: frame_system::{Pallet, Call, Storage, Config, Event<T>},
		Timestamp: pallet_timestamp::{Pallet, Call, Storage, Inherent},
		Sudo: pallet_sudo::{Pallet, Call, Storage, Config<T>, Event<T>},
		RandomnessCollectiveFlip: pallet_randomness_collective_flip::{Pallet, Storage},
		TransactionPayment: pallet_transaction_payment::{Pallet, Storage, Event<T>},

		ParachainSystem: cumulus_pallet_parachain_system::{Pallet, Call, Storage, Inherent, Event<T>} = 20,
		ParachainInfo: parachain_info::{Pallet, Storage, Config} = 21,

		Balances: pallet_balances::{Pallet, Call, Storage, Config<T>, Event<T>} = 30,

		Aura: pallet_aura::{Pallet, Config<T>},
		AuraExt: cumulus_pallet_aura_ext::{Pallet, Config},

		// XCM helpers.
		XcmpQueue: cumulus_pallet_xcmp_queue::{Pallet, Call, Storage, Event<T>} = 50,
		PolkadotXcm: pallet_xcm::{Pallet, Call, Event<T>, Origin} = 51,
		CumulusXcm: cumulus_pallet_xcm::{Pallet, Call, Event<T>, Origin} = 52,
		DmpQueue: cumulus_pallet_dmp_queue::{Pallet, Call, Storage, Event<T>} = 53,

		// Millau bridge modules.
		BridgeRelayers: pallet_bridge_relayers::{Pallet, Call, Storage, Event<T>},
		BridgeMillauGrandpa: pallet_bridge_grandpa::{Pallet, Call, Storage},
		BridgeMillauMessages: pallet_bridge_messages::{Pallet, Call, Storage, Event<T>, Config<T>},
	}
);

impl_runtime_apis! {
	impl sp_api::Core<Block> for Runtime {
		fn version() -> RuntimeVersion {
			VERSION
		}

		fn execute_block(block: Block) {
			Executive::execute_block(block)
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
		fn apply_extrinsic(
			extrinsic: <Block as BlockT>::Extrinsic,
		) -> ApplyExtrinsicResult {
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

	impl sp_session::SessionKeys<Block> for Runtime {
		fn decode_session_keys(
			encoded: Vec<u8>,
		) -> Option<Vec<(Vec<u8>, KeyTypeId)>> {
			SessionKeys::decode_into_raw_public_keys(&encoded)
		}

		fn generate_session_keys(seed: Option<Vec<u8>>) -> Vec<u8> {
			SessionKeys::generate(seed)
		}
	}

	impl sp_consensus_aura::AuraApi<Block, AuraId> for Runtime {
		fn slot_duration() -> sp_consensus_aura::SlotDuration {
			sp_consensus_aura::SlotDuration::from_millis(Aura::slot_duration())
		}

		fn authorities() -> Vec<AuraId> {
			Aura::authorities().to_vec()
		}
	}

	impl cumulus_primitives_core::CollectCollationInfo<Block> for Runtime {
		fn collect_collation_info(header: &<Block as BlockT>::Header) -> cumulus_primitives_core::CollationInfo {
			ParachainSystem::collect_collation_info(header)
		}
	}

	impl frame_system_rpc_runtime_api::AccountNonceApi<Block, AccountId, Index> for Runtime {
		fn account_nonce(account: AccountId) -> Index {
			System::account_nonce(account)
		}
	}

	impl pallet_transaction_payment_rpc_runtime_api::TransactionPaymentApi<Block, Balance> for Runtime {
		fn query_info(
			uxt: <Block as BlockT>::Extrinsic,
			len: u32,
		) -> pallet_transaction_payment_rpc_runtime_api::RuntimeDispatchInfo<Balance> {
			TransactionPayment::query_info(uxt, len)
		}
		fn query_fee_details(
			uxt: <Block as BlockT>::Extrinsic,
			len: u32,
		) -> pallet_transaction_payment::FeeDetails<Balance> {
			TransactionPayment::query_fee_details(uxt, len)
		}
	}

	impl bp_millau::MillauFinalityApi<Block> for Runtime {
		fn best_finalized() -> Option<HeaderId<bp_millau::Hash, bp_millau::BlockNumber>> {
			BridgeMillauGrandpa::best_finalized().map(|header| header.id())
		}
	}

	impl bp_millau::ToMillauOutboundLaneApi<Block, Balance, ToMillauMessagePayload> for Runtime {
		fn estimate_message_delivery_and_dispatch_fee(
			_lane_id: bp_messages::LaneId,
			payload: ToMillauMessagePayload,
			millau_to_this_conversion_rate: Option<FixedU128>,
		) -> Option<Balance> {
			estimate_message_dispatch_and_delivery_fee::<WithMillauMessageBridge>(
				&payload,
				WithMillauMessageBridge::RELAYER_FEE_PERCENT,
				millau_to_this_conversion_rate,
			).ok()
		}

		fn message_details(
			lane: bp_messages::LaneId,
			begin: bp_messages::MessageNonce,
			end: bp_messages::MessageNonce,
		) -> Vec<bp_messages::OutboundMessageDetails<Balance>> {
			bridge_runtime_common::messages_api::outbound_message_details::<
				Runtime,
				WithMillauMessagesInstance,
			>(lane, begin, end)
		}
	}

	impl bp_millau::FromMillauInboundLaneApi<Block, bp_millau::Balance> for Runtime {
		fn message_details(
			lane: bp_messages::LaneId,
			messages: Vec<(bp_messages::MessagePayload, bp_messages::OutboundMessageDetails<bp_millau::Balance>)>,
		) -> Vec<bp_messages::InboundMessageDetails> {
			bridge_runtime_common::messages_api::inbound_message_details::<
				Runtime,
				WithMillauMessagesInstance,
			>(lane, messages)
		}
	}

	#[cfg(feature = "runtime-benchmarks")]
	impl frame_benchmarking::Benchmark<Block> for Runtime {
		fn benchmark_metadata(_extra: bool) -> (
			Vec<frame_benchmarking::BenchmarkList>,
			Vec<frame_support::traits::StorageInfo>,
		) {
			todo!("TODO: fix or remove")
		}

		fn dispatch_benchmark(
			config: frame_benchmarking::BenchmarkConfig
		) -> Result<Vec<frame_benchmarking::BenchmarkBatch>, sp_runtime::RuntimeString> {
			use frame_benchmarking::{Benchmarking, BenchmarkBatch, add_benchmark, TrackedStorageKey};

			use frame_system_benchmarking::Pallet as SystemBench;
			impl frame_system_benchmarking::Config for Runtime {}

			let whitelist: Vec<TrackedStorageKey> = vec![
				// Block Number
				hex_literal::hex!("26aa394eea5630e07c48ae0c9558cef702a5c1b19ab7a04f536c519aca4983ac").to_vec().into(),
				// Total Issuance
				hex_literal::hex!("c2261276cc9d1f8598ea4b6a74b15c2f57c875e4cff74148e4628f264b974c80").to_vec().into(),
				// Execution Phase
				hex_literal::hex!("26aa394eea5630e07c48ae0c9558cef7ff553b5a9862a516939d82b3d3d8661a").to_vec().into(),
				// Event Count
				hex_literal::hex!("26aa394eea5630e07c48ae0c9558cef70a98fdbe9ce6c55837576c60c7af3850").to_vec().into(),
				// System Events
				hex_literal::hex!("26aa394eea5630e07c48ae0c9558cef780d41e5e16056765bc8461851072c9d7").to_vec().into(),
			];

			let mut batches = Vec::<BenchmarkBatch>::new();
			let params = (&config, &whitelist);

			add_benchmark!(params, batches, frame_system, SystemBench::<Runtime>);
			add_benchmark!(params, batches, pallet_balances, Balances);
			add_benchmark!(params, batches, pallet_timestamp, Timestamp);

			Ok(batches)
		}
	}
}

struct CheckInherents;

impl cumulus_pallet_parachain_system::CheckInherents<Block> for CheckInherents {
	fn check_inherents(
		block: &Block,
		relay_state_proof: &cumulus_pallet_parachain_system::RelayChainStateProof,
	) -> sp_inherents::CheckInherentsResult {
		let relay_chain_slot = relay_state_proof
			.read_slot()
			.expect("Could not read the relay chain slot from the proof");

		let inherent_data =
			cumulus_primitives_timestamp::InherentDataProvider::from_relay_chain_slot_and_duration(
				relay_chain_slot,
				sp_std::time::Duration::from_secs(6),
			)
			.create_inherent_data()
			.expect("Could not create the timestamp inherent data");

		inherent_data.check_extrinsics(block)
	}
}

cumulus_pallet_parachain_system::register_validate_block!(
	Runtime = Runtime,
	BlockExecutor = cumulus_pallet_aura_ext::BlockExecutor::<Runtime, Executive>,
	CheckInherents = CheckInherents,
);

#[cfg(test)]
mod tests {
	use super::*;
	use bp_messages::{
		target_chain::{DispatchMessage, DispatchMessageData, MessageDispatch},
		MessageKey,
	};
	use bp_runtime::messages::MessageDispatchResult;
	use bridge_runtime_common::messages::target::FromBridgedChainMessageDispatch;
	use codec::Encode;

	fn new_test_ext() -> sp_io::TestExternalities {
		sp_io::TestExternalities::new(
			frame_system::GenesisConfig::default().build_storage::<Runtime>().unwrap(),
		)
	}

	#[test]
	fn xcm_messages_to_millau_are_sent() {
		new_test_ext().execute_with(|| {
			// the encoded message (origin ++ xcm) is 0x010109020419A8
			let dest = (Parent, X1(GlobalConsensus(MillauNetwork::get())));
			let xcm: Xcm<()> = vec![Instruction::Trap(42)].into();

			let send_result = send_xcm::<XcmRouter>(dest.into(), xcm);
			let expected_fee = MultiAssets::from((Here, Fungibility::Fungible(4_259_858_152_u128)));
			let expected_hash =
				([0u8, 0u8, 0u8, 0u8], 1u64).using_encoded(sp_io::hashing::blake2_256);
			assert_eq!(send_result, Ok((expected_hash, expected_fee)),);
		})
	}

	#[test]
	fn xcm_messages_from_millau_are_dispatched() {
		type XcmExecutor = xcm_executor::XcmExecutor<XcmConfig>;
		type MessageDispatcher = FromBridgedChainMessageDispatch<
			WithMillauMessageBridge,
			XcmExecutor,
			XcmWeigher,
			frame_support::traits::ConstU64<BASE_XCM_WEIGHT>,
		>;

		new_test_ext().execute_with(|| {
			let location: MultiLocation =
				(Parent, X1(GlobalConsensus(MillauNetwork::get()))).into();
			let xcm: Xcm<Call> = vec![Instruction::Trap(42)].into();

			let mut incoming_message = DispatchMessage {
				key: MessageKey { lane_id: [0, 0, 0, 0], nonce: 1 },
				data: DispatchMessageData { payload: Ok((location, xcm).into()), fee: 0 },
			};

			let dispatch_weight = MessageDispatcher::dispatch_weight(&mut incoming_message);
			assert_eq!(dispatch_weight, 1_000_000_000);

			let dispatch_result =
				MessageDispatcher::dispatch(&AccountId::from([0u8; 32]), incoming_message);
			assert_eq!(
				dispatch_result,
				MessageDispatchResult {
					dispatch_result: true,
					unspent_weight: 0,
					dispatch_fee_paid_during_dispatch: false,
				}
			);
		})
	}
}
