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

use crate::HeaderIdProvider;
use codec::{Decode, Encode};
use frame_support::{weights::Weight, Parameter};
use num_traits::{AsPrimitive, Bounded, CheckedSub, Saturating, SaturatingAdd, Zero};
use sp_runtime::{
	traits::{
		AtLeast32Bit, AtLeast32BitUnsigned, Hash as HashT, Header as HeaderT, MaybeDisplay,
		MaybeMallocSizeOf, MaybeSerialize, MaybeSerializeDeserialize, Member, SimpleBitOps, Verify,
	},
	FixedPointOperand,
};
use sp_std::{convert::TryFrom, fmt::Debug, hash::Hash, str::FromStr, vec, vec::Vec};

/// Chain call, that is either SCALE-encoded, or decoded.
#[derive(Debug, Clone, PartialEq)]
pub enum EncodedOrDecodedCall<ChainCall> {
	/// The call that is SCALE-encoded.
	///
	/// This variant is used when we the chain runtime is not bundled with the relay, but
	/// we still need the represent call in some RPC calls or transactions.
	Encoded(Vec<u8>),
	/// The decoded call.
	Decoded(ChainCall),
}

impl<ChainCall: Clone + Decode> EncodedOrDecodedCall<ChainCall> {
	/// Returns decoded call.
	pub fn to_decoded(&self) -> Result<ChainCall, codec::Error> {
		match self {
			Self::Encoded(ref encoded_call) =>
				ChainCall::decode(&mut &encoded_call[..]).map_err(Into::into),
			Self::Decoded(ref decoded_call) => Ok(decoded_call.clone()),
		}
	}

	/// Converts self to decoded call.
	pub fn into_decoded(self) -> Result<ChainCall, codec::Error> {
		match self {
			Self::Encoded(encoded_call) =>
				ChainCall::decode(&mut &encoded_call[..]).map_err(Into::into),
			Self::Decoded(decoded_call) => Ok(decoded_call),
		}
	}
}

impl<ChainCall> From<ChainCall> for EncodedOrDecodedCall<ChainCall> {
	fn from(call: ChainCall) -> EncodedOrDecodedCall<ChainCall> {
		EncodedOrDecodedCall::Decoded(call)
	}
}

impl<ChainCall: Decode> Decode for EncodedOrDecodedCall<ChainCall> {
	fn decode<I: codec::Input>(input: &mut I) -> Result<Self, codec::Error> {
		// having encoded version is better than decoded, because decoding isn't required
		// everywhere and for mocked calls it may lead to **unneeded** errors
		match input.remaining_len()? {
			Some(remaining_len) => {
				let mut encoded_call = vec![0u8; remaining_len];
				input.read(&mut encoded_call)?;
				Ok(EncodedOrDecodedCall::Encoded(encoded_call))
			},
			None => Ok(EncodedOrDecodedCall::Decoded(ChainCall::decode(input)?)),
		}
	}
}

impl<ChainCall: Encode> Encode for EncodedOrDecodedCall<ChainCall> {
	fn encode(&self) -> Vec<u8> {
		match *self {
			Self::Encoded(ref encoded_call) => encoded_call.clone(),
			Self::Decoded(ref decoded_call) => decoded_call.encode(),
		}
	}
}

/// Minimal Substrate-based chain representation that may be used from no_std environment.
pub trait Chain: Send + Sync + 'static {
	/// A type that fulfills the abstract idea of what a Substrate block number is.
	// Constraits come from the associated Number type of `sp_runtime::traits::Header`
	// See here for more info:
	// https://crates.parity.io/sp_runtime/traits/trait.Header.html#associatedtype.Number
	//
	// Note that the `AsPrimitive<usize>` trait is required by the GRANDPA justification
	// verifier, and is not usually part of a Substrate Header's Number type.
	type BlockNumber: Parameter
		+ Member
		+ MaybeSerializeDeserialize
		+ Hash
		+ Copy
		+ Default
		+ MaybeDisplay
		+ AtLeast32BitUnsigned
		+ FromStr
		+ MaybeMallocSizeOf
		+ AsPrimitive<usize>
		+ Default
		+ Saturating
		// original `sp_runtime::traits::Header::BlockNumber` doesn't have this trait, but
		// `sp_runtime::generic::Era` requires block number -> `u64` conversion.
		+ Into<u64>;

	/// A type that fulfills the abstract idea of what a Substrate hash is.
	// Constraits come from the associated Hash type of `sp_runtime::traits::Header`
	// See here for more info:
	// https://crates.parity.io/sp_runtime/traits/trait.Header.html#associatedtype.Hash
	type Hash: Parameter
		+ Member
		+ MaybeSerializeDeserialize
		+ Hash
		+ Ord
		+ Copy
		+ MaybeDisplay
		+ Default
		+ SimpleBitOps
		+ AsRef<[u8]>
		+ AsMut<[u8]>
		+ MaybeMallocSizeOf;

	/// A type that fulfills the abstract idea of what a Substrate hasher (a type
	/// that produces hashes) is.
	// Constraits come from the associated Hashing type of `sp_runtime::traits::Header`
	// See here for more info:
	// https://crates.parity.io/sp_runtime/traits/trait.Header.html#associatedtype.Hashing
	type Hasher: HashT<Output = Self::Hash>;

	/// A type that fulfills the abstract idea of what a Substrate header is.
	// See here for more info:
	// https://crates.parity.io/sp_runtime/traits/trait.Header.html
	type Header: Parameter
		+ HeaderT<Number = Self::BlockNumber, Hash = Self::Hash>
		+ HeaderIdProvider<Self::Header>
		+ MaybeSerializeDeserialize;

	/// The user account identifier type for the runtime.
	type AccountId: Parameter + Member + MaybeSerializeDeserialize + Debug + MaybeDisplay + Ord;
	/// Balance of an account in native tokens.
	///
	/// The chain may support multiple tokens, but this particular type is for token that is used
	/// to pay for transaction dispatch, to reward different relayers (headers, messages), etc.
	type Balance: AtLeast32BitUnsigned
		+ FixedPointOperand
		+ Parameter
		+ Member
		+ MaybeSerializeDeserialize
		+ Clone
		+ Copy
		+ Bounded
		+ CheckedSub
		+ PartialOrd
		+ SaturatingAdd
		+ Zero
		+ TryFrom<sp_core::U256>;
	/// Index of a transaction used by the chain.
	type Index: Parameter
		+ Member
		+ MaybeSerialize
		+ Debug
		+ Default
		+ MaybeDisplay
		+ MaybeSerializeDeserialize
		+ AtLeast32Bit
		+ Copy;
	/// Signature type, used on this chain.
	type Signature: Parameter + Verify;

	/// Get the maximum size (in bytes) of a Normal extrinsic at this chain.
	fn max_extrinsic_size() -> u32;
	/// Get the maximum weight (compute time) that a Normal extrinsic at this chain can use.
	fn max_extrinsic_weight() -> Weight;
}

/// Block number used by the chain.
pub type BlockNumberOf<C> = <C as Chain>::BlockNumber;

/// Hash type used by the chain.
pub type HashOf<C> = <C as Chain>::Hash;

/// Hasher type used by the chain.
pub type HasherOf<C> = <C as Chain>::Hasher;

/// Header type used by the chain.
pub type HeaderOf<C> = <C as Chain>::Header;

/// Account id type used by the chain.
pub type AccountIdOf<C> = <C as Chain>::AccountId;

/// Balance type used by the chain.
pub type BalanceOf<C> = <C as Chain>::Balance;

/// Transaction index type used by the chain.
pub type IndexOf<C> = <C as Chain>::Index;

/// Signature type used by the chain.
pub type SignatureOf<C> = <C as Chain>::Signature;

/// Account public type used by the chain.
pub type AccountPublicOf<C> = <SignatureOf<C> as Verify>::Signer;

/// Transaction era used by the chain.
pub type TransactionEraOf<C> = crate::TransactionEra<BlockNumberOf<C>, HashOf<C>>;

/// Convenience macro that declares bridge finality runtime apis and related constants for a chain.
/// This includes:
/// - chain-specific bridge runtime APIs:
///     - `<ThisChain>FinalityApi`
/// - constants that are stringified names of runtime API methods:
///     - `BEST_FINALIZED_<THIS_CHAIN>_HEADER_METHOD`
/// The name of the chain has to be specified in snake case (e.g. `rialto_parachain`).
#[macro_export]
macro_rules! decl_bridge_finality_runtime_apis {
	($chain: ident) => {
		bp_runtime::paste::item! {
			mod [<$chain _finality_api>] {
				use super::*;

				/// Name of the `<ThisChain>FinalityApi::best_finalized` runtime method.
				pub const [<BEST_FINALIZED_ $chain:upper _HEADER_METHOD>]: &str =
					stringify!([<$chain:camel FinalityApi_best_finalized>]);

				sp_api::decl_runtime_apis! {
					/// API for querying information about the finalized chain headers.
					///
					/// This API is implemented by runtimes that are receiving messages from this chain, not by this
					/// chain's runtime itself.
					pub trait [<$chain:camel FinalityApi>] {
						/// Returns number and hash of the best finalized header known to the bridge module.
						fn best_finalized() -> Option<bp_runtime::HeaderId<Hash, BlockNumber>>;
					}
				}
			}

			pub use [<$chain _finality_api>]::*;
		}
	};
}

/// Convenience macro that declares bridge messages runtime apis and related constants for a chain.
/// This includes:
/// - chain-specific bridge runtime APIs:
///     - `To<ThisChain>OutboundLaneApi`
///     - `From<ThisChain>InboundLaneApi`
/// - constants that are stringified names of runtime API methods:
///     - `TO_<THIS_CHAIN>_ESTIMATE_MESSAGE_FEE_METHOD`
///     - `TO_<THIS_CHAIN>_MESSAGE_DETAILS_METHOD`
///     - `FROM_<THIS_CHAIN>_MESSAGE_DETAILS_METHOD`,
/// The name of the chain has to be specified in snake case (e.g. `rialto_parachain`).
#[macro_export]
macro_rules! decl_bridge_messages_runtime_apis {
	($chain: ident) => {
		bp_runtime::paste::item! {
			mod [<$chain _messages_api>] {
				use super::*;

				/// Name of the `To<ThisChain>OutboundLaneApi::estimate_message_delivery_and_dispatch_fee` runtime
				/// method.
				pub const [<TO_ $chain:upper _ESTIMATE_MESSAGE_FEE_METHOD>]: &str =
					stringify!([<To $chain:camel OutboundLaneApi_estimate_message_delivery_and_dispatch_fee>]);
				/// Name of the `To<ThisChain>OutboundLaneApi::message_details` runtime method.
				pub const [<TO_ $chain:upper _MESSAGE_DETAILS_METHOD>]: &str =
					stringify!([<To $chain:camel OutboundLaneApi_message_details>]);

				/// Name of the `From<ThisChain>InboundLaneApi::message_details` runtime method.
				pub const [<FROM_ $chain:upper _MESSAGE_DETAILS_METHOD>]: &str =
					stringify!([<From $chain:camel InboundLaneApi_message_details>]);

				sp_api::decl_runtime_apis! {
					/// Outbound message lane API for messages that are sent to this chain.
					///
					/// This API is implemented by runtimes that are receiving messages from this chain, not by this
					/// chain's runtime itself.
					pub trait [<To $chain:camel OutboundLaneApi>]<OutboundMessageFee: Parameter, OutboundPayload: Parameter> {
						/// Estimate message delivery and dispatch fee that needs to be paid by the sender on
						/// this chain.
						///
						/// Returns `None` if message is too expensive to be sent to this chain from the bridged chain.
						///
						/// Please keep in mind that this method returns the lowest message fee required for message
						/// to be accepted to the lane. It may be a good idea to pay a bit over this price to account
						/// for future exchange rate changes and guarantee that relayer would deliver your message
						/// to the target chain.
						fn estimate_message_delivery_and_dispatch_fee(
							lane_id: LaneId,
							payload: OutboundPayload,
							[<$chain:lower _to_this_conversion_rate>]: Option<FixedU128>,
						) -> Option<OutboundMessageFee>;
						/// Returns dispatch weight, encoded payload size and delivery+dispatch fee of all
						/// messages in given inclusive range.
						///
						/// If some (or all) messages are missing from the storage, they'll also will
						/// be missing from the resulting vector. The vector is ordered by the nonce.
						fn message_details(
							lane: LaneId,
							begin: MessageNonce,
							end: MessageNonce,
						) -> Vec<OutboundMessageDetails<OutboundMessageFee>>;
					}

					/// Inbound message lane API for messages sent by this chain.
					///
					/// This API is implemented by runtimes that are receiving messages from this chain, not by this
					/// chain's runtime itself.
					///
					/// Entries of the resulting vector are matching entries of the `messages` vector. Entries of the
					/// `messages` vector may (and need to) be read using `To<ThisChain>OutboundLaneApi::message_details`.
					pub trait [<From $chain:camel InboundLaneApi>]<InboundMessageFee: Parameter> {
						/// Return details of given inbound messages.
						fn message_details(
							lane: LaneId,
							messages: Vec<(MessagePayload, OutboundMessageDetails<InboundMessageFee>)>,
						) -> Vec<InboundMessageDetails>;
					}
				}
			}

			pub use [<$chain _messages_api>]::*;
		}
	}
}

/// Convenience macro that declares bridge finality runtime apis, bridge messages runtime apis
/// and related constants for a chain.
/// The name of the chain has to be specified in snake case (e.g. `rialto_parachain`).
#[macro_export]
macro_rules! decl_bridge_runtime_apis {
	($chain: ident) => {
		bp_runtime::decl_bridge_finality_runtime_apis!($chain);
		bp_runtime::decl_bridge_messages_runtime_apis!($chain);
	};
}
