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

//! Primitives of messages module.

#![cfg_attr(not(feature = "std"), no_std)]
// RuntimeApi generated functions
#![allow(clippy::too_many_arguments)]

use bitvec::prelude::*;
use bp_runtime::{messages::DispatchFeePayment, BasicOperatingMode, OperatingMode};
use codec::{Decode, Encode, MaxEncodedLen};
use frame_support::RuntimeDebug;
use scale_info::TypeInfo;
use sp_std::{collections::vec_deque::VecDeque, prelude::*};

pub mod source_chain;
pub mod storage_keys;
pub mod target_chain;

// Weight is reexported to avoid additional frame-support dependencies in related crates.
pub use frame_support::weights::Weight;

/// Messages pallet operating mode.
#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub enum MessagesOperatingMode {
	/// Basic operating mode (Normal/Halted)
	Basic(BasicOperatingMode),
	/// The pallet is not accepting outbound messages. Inbound messages and receiving proofs
	/// are still accepted.
	///
	/// This mode may be used e.g. when bridged chain expects upgrade. Then to avoid dispatch
	/// failures, the pallet owner may stop accepting new messages, while continuing to deliver
	/// queued messages to the bridged chain. Once upgrade is completed, the mode may be switched
	/// back to `Normal`.
	RejectingOutboundMessages,
}

impl Default for MessagesOperatingMode {
	fn default() -> Self {
		MessagesOperatingMode::Basic(BasicOperatingMode::Normal)
	}
}

impl OperatingMode for MessagesOperatingMode {
	fn is_halted(&self) -> bool {
		match self {
			Self::Basic(operating_mode) => operating_mode.is_halted(),
			_ => false,
		}
	}
}

/// Messages pallet parameter.
pub trait Parameter: frame_support::Parameter {
	/// Save parameter value in the runtime storage.
	fn save(&self);
}

impl Parameter for () {
	fn save(&self) {}
}

/// Lane identifier.
pub type LaneId = [u8; 4];

/// Message nonce. Valid messages will never have 0 nonce.
pub type MessageNonce = u64;

/// Message id as a tuple.
pub type BridgeMessageId = (LaneId, MessageNonce);

/// Opaque message payload. We only decode this payload when it is dispatched.
pub type MessagePayload = Vec<u8>;

/// Message key (unique message identifier) as it is stored in the storage.
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct MessageKey {
	/// ID of the message lane.
	pub lane_id: LaneId,
	/// Message nonce.
	pub nonce: MessageNonce,
}

/// Message data as it is stored in the storage.
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct MessageData<Fee> {
	/// Message payload.
	pub payload: MessagePayload,
	/// Message delivery and dispatch fee, paid by the submitter.
	pub fee: Fee,
}

/// Message as it is stored in the storage.
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct Message<Fee> {
	/// Message key.
	pub key: MessageKey,
	/// Message data.
	pub data: MessageData<Fee>,
}

/// Inbound lane data.
#[derive(Encode, Decode, Clone, RuntimeDebug, PartialEq, Eq, TypeInfo)]
pub struct InboundLaneData<RelayerId> {
	/// Identifiers of relayers and messages that they have delivered to this lane (ordered by
	/// message nonce).
	///
	/// This serves as a helper storage item, to allow the source chain to easily pay rewards
	/// to the relayers who successfully delivered messages to the target chain (inbound lane).
	///
	/// It is guaranteed to have at most N entries, where N is configured at the module level.
	/// If there are N entries in this vec, then:
	/// 1) all incoming messages are rejected if they're missing corresponding
	/// `proof-of(outbound-lane.state)`; 2) all incoming messages are rejected if
	/// `proof-of(outbound-lane.state).last_delivered_nonce` is    equal to
	/// `self.last_confirmed_nonce`. Given what is said above, all nonces in this queue are in
	/// range: `(self.last_confirmed_nonce; self.last_delivered_nonce()]`.
	///
	/// When a relayer sends a single message, both of MessageNonces are the same.
	/// When relayer sends messages in a batch, the first arg is the lowest nonce, second arg the
	/// highest nonce. Multiple dispatches from the same relayer are allowed.
	pub relayers: VecDeque<UnrewardedRelayer<RelayerId>>,

	/// Nonce of the last message that
	/// a) has been delivered to the target (this) chain and
	/// b) the delivery has been confirmed on the source chain
	///
	/// that the target chain knows of.
	///
	/// This value is updated indirectly when an `OutboundLane` state of the source
	/// chain is received alongside with new messages delivery.
	pub last_confirmed_nonce: MessageNonce,
}

impl<RelayerId> Default for InboundLaneData<RelayerId> {
	fn default() -> Self {
		InboundLaneData { relayers: VecDeque::new(), last_confirmed_nonce: 0 }
	}
}

impl<RelayerId> InboundLaneData<RelayerId> {
	/// Returns approximate size of the struct, given a number of entries in the `relayers` set and
	/// size of each entry.
	///
	/// Returns `None` if size overflows `usize` limits.
	pub fn encoded_size_hint(relayers_entries: usize, messages_count: usize) -> Option<usize>
	where
		RelayerId: MaxEncodedLen,
	{
		let message_nonce_size = MessageNonce::max_encoded_len();
		let relayer_id_encoded_size = RelayerId::max_encoded_len();
		let relayers_entry_size = relayer_id_encoded_size.checked_add(2 * message_nonce_size)?;
		let relayers_size = relayers_entries.checked_mul(relayers_entry_size)?;
		let dispatch_results_per_byte = 8;
		let dispatch_result_size =
			sp_std::cmp::max(relayers_entries, messages_count / dispatch_results_per_byte);
		relayers_size
			.checked_add(message_nonce_size)
			.and_then(|result| result.checked_add(dispatch_result_size))
	}

	/// Returns the approximate size of the struct as u32, given a number of entries in the
	/// `relayers` set and the size of each entry.
	///
	/// Returns `u32::MAX` if size overflows `u32` limits.
	pub fn encoded_size_hint_u32(relayers_entries: usize, messages_count: usize) -> u32
	where
		RelayerId: MaxEncodedLen,
	{
		Self::encoded_size_hint(relayers_entries, messages_count)
			.and_then(|x| u32::try_from(x).ok())
			.unwrap_or(u32::MAX)
	}

	/// Nonce of the last message that has been delivered to this (target) chain.
	pub fn last_delivered_nonce(&self) -> MessageNonce {
		self.relayers
			.back()
			.map(|entry| entry.messages.end)
			.unwrap_or(self.last_confirmed_nonce)
	}
}

/// Outbound message details, returned by runtime APIs.
#[derive(Clone, Encode, Decode, RuntimeDebug, PartialEq, Eq)]
pub struct OutboundMessageDetails<OutboundMessageFee> {
	/// Nonce assigned to the message.
	pub nonce: MessageNonce,
	/// Message dispatch weight.
	///
	/// Depending on messages pallet configuration, it may be declared by the message submitter,
	/// computed automatically or just be zero if dispatch fee is paid at the target chain.
	pub dispatch_weight: Weight,
	/// Size of the encoded message.
	pub size: u32,
	/// Delivery+dispatch fee paid by the message submitter at the source chain.
	pub delivery_and_dispatch_fee: OutboundMessageFee,
	/// Where the fee for dispatching message is paid?
	pub dispatch_fee_payment: DispatchFeePayment,
}

/// Inbound message details, returned by runtime APIs.
#[derive(Clone, Encode, Decode, RuntimeDebug, PartialEq, Eq)]
pub struct InboundMessageDetails {
	/// Computed message dispatch weight.
	///
	/// Runtime API guarantees that it will match the value, returned by
	/// `target_chain::MessageDispatch::dispatch_weight`. This means that if the runtime
	/// has failed to decode the message, it will be zero - that's because `undecodable`
	/// message cannot be dispatched.
	pub dispatch_weight: Weight,
}

/// Bit vector of message dispatch results.
pub type DispatchResultsBitVec = BitVec<u8, Msb0>;

/// Unrewarded relayer entry stored in the inbound lane data.
///
/// This struct represents a continuous range of messages that have been delivered by the same
/// relayer and whose confirmations are still pending.
#[derive(Encode, Decode, Clone, RuntimeDebug, PartialEq, Eq, TypeInfo)]
pub struct UnrewardedRelayer<RelayerId> {
	/// Identifier of the relayer.
	pub relayer: RelayerId,
	/// Messages range, delivered by this relayer.
	pub messages: DeliveredMessages,
}

/// Delivered messages with their dispatch result.
#[derive(Clone, Default, Encode, Decode, RuntimeDebug, PartialEq, Eq, TypeInfo)]
pub struct DeliveredMessages {
	/// Nonce of the first message that has been delivered (inclusive).
	pub begin: MessageNonce,
	/// Nonce of the last message that has been delivered (inclusive).
	pub end: MessageNonce,
	/// Dispatch result (`false`/`true`), returned by the message dispatcher for every
	/// message in the `[begin; end]` range. See `dispatch_result` field of the
	/// `bp_runtime::messages::MessageDispatchResult` structure for more information.
	pub dispatch_results: DispatchResultsBitVec,
}

impl DeliveredMessages {
	/// Create new `DeliveredMessages` struct that confirms delivery of single nonce with given
	/// dispatch result.
	pub fn new(nonce: MessageNonce, dispatch_result: bool) -> Self {
		let mut dispatch_results = BitVec::with_capacity(1);
		dispatch_results.push(dispatch_result);
		DeliveredMessages { begin: nonce, end: nonce, dispatch_results }
	}

	/// Return total count of delivered messages.
	pub fn total_messages(&self) -> MessageNonce {
		if self.end >= self.begin {
			self.end - self.begin + 1
		} else {
			0
		}
	}

	/// Note new dispatched message.
	pub fn note_dispatched_message(&mut self, dispatch_result: bool) {
		self.end += 1;
		self.dispatch_results.push(dispatch_result);
	}

	/// Returns true if delivered messages contain message with given nonce.
	pub fn contains_message(&self, nonce: MessageNonce) -> bool {
		(self.begin..=self.end).contains(&nonce)
	}

	/// Get dispatch result flag by message nonce.
	///
	/// Dispatch result flag must be interpreted using the knowledge of dispatch mechanism
	/// at the target chain. See `dispatch_result` field of the
	/// `bp_runtime::messages::MessageDispatchResult` structure for more information.
	///
	/// Panics if message nonce is not in the `begin..=end` range. Typically you'll first
	/// check if message is within the range by calling `contains_message`.
	pub fn message_dispatch_result(&self, nonce: MessageNonce) -> bool {
		const INVALID_NONCE: &str = "Invalid nonce used to index dispatch_results";

		let index = nonce.checked_sub(self.begin).expect(INVALID_NONCE) as usize;
		*self.dispatch_results.get(index).expect(INVALID_NONCE)
	}
}

/// Gist of `InboundLaneData::relayers` field used by runtime APIs.
#[derive(Clone, Default, Encode, Decode, RuntimeDebug, PartialEq, Eq, TypeInfo)]
pub struct UnrewardedRelayersState {
	/// Number of entries in the `InboundLaneData::relayers` set.
	pub unrewarded_relayer_entries: MessageNonce,
	/// Number of messages in the oldest entry of `InboundLaneData::relayers`. This is the
	/// minimal number of reward proofs required to push out this entry from the set.
	pub messages_in_oldest_entry: MessageNonce,
	/// Total number of messages in the relayers vector.
	pub total_messages: MessageNonce,
	/// Nonce of the latest message that has been delivered to the target chain.
	///
	/// This corresponds to the result of the `InboundLaneData::last_delivered_nonce` call
	/// at the bridged chain.
	pub last_delivered_nonce: MessageNonce,
}

/// Outbound lane data.
#[derive(Encode, Decode, Clone, RuntimeDebug, PartialEq, Eq, TypeInfo, MaxEncodedLen)]
pub struct OutboundLaneData {
	/// Nonce of the oldest message that we haven't yet pruned. May point to not-yet-generated
	/// message if all sent messages are already pruned.
	pub oldest_unpruned_nonce: MessageNonce,
	/// Nonce of the latest message, received by bridged chain.
	pub latest_received_nonce: MessageNonce,
	/// Nonce of the latest message, generated by us.
	pub latest_generated_nonce: MessageNonce,
}

impl Default for OutboundLaneData {
	fn default() -> Self {
		OutboundLaneData {
			// it is 1 because we're pruning everything in [oldest_unpruned_nonce;
			// latest_received_nonce]
			oldest_unpruned_nonce: 1,
			latest_received_nonce: 0,
			latest_generated_nonce: 0,
		}
	}
}

/// Returns total number of messages in the `InboundLaneData::relayers` vector.
///
/// Returns `None` if there are more messages that `MessageNonce` may fit (i.e. `MessageNonce + 1`).
pub fn total_unrewarded_messages<RelayerId>(
	relayers: &VecDeque<UnrewardedRelayer<RelayerId>>,
) -> Option<MessageNonce> {
	match (relayers.front(), relayers.back()) {
		(Some(front), Some(back)) => {
			if let Some(difference) = back.messages.end.checked_sub(front.messages.begin) {
				difference.checked_add(1)
			} else {
				Some(0)
			}
		},
		_ => Some(0),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn total_unrewarded_messages_does_not_overflow() {
		assert_eq!(
			total_unrewarded_messages(
				&vec![
					UnrewardedRelayer { relayer: 1, messages: DeliveredMessages::new(0, true) },
					UnrewardedRelayer {
						relayer: 2,
						messages: DeliveredMessages::new(MessageNonce::MAX, true)
					},
				]
				.into_iter()
				.collect()
			),
			None,
		);
	}

	#[test]
	fn inbound_lane_data_returns_correct_hint() {
		let test_cases = vec![
			// single relayer, multiple messages
			(1, 128u8),
			// multiple relayers, single message per relayer
			(128u8, 128u8),
			// several messages per relayer
			(13u8, 128u8),
		];
		for (relayer_entries, messages_count) in test_cases {
			let expected_size =
				InboundLaneData::<u8>::encoded_size_hint(relayer_entries as _, messages_count as _);
			let actual_size = InboundLaneData {
				relayers: (1u8..=relayer_entries)
					.map(|i| {
						let mut entry = UnrewardedRelayer {
							relayer: i,
							messages: DeliveredMessages::new(i as _, true),
						};
						entry.messages.dispatch_results = bitvec![
							u8, Msb0;
							1;
							(messages_count / relayer_entries) as _
						];
						entry
					})
					.collect(),
				last_confirmed_nonce: messages_count as _,
			}
			.encode()
			.len();
			let difference = (expected_size.unwrap() as f64 - actual_size as f64).abs();
			assert!(
				difference / (std::cmp::min(actual_size, expected_size.unwrap() as usize) as f64) < 0.1,
				"Too large difference between actual ({}) and expected ({:?}) inbound lane data size. Test case: {}+{}",
				actual_size,
				expected_size,
				relayer_entries,
				messages_count,
			);
		}
	}

	#[test]
	fn message_dispatch_result_works() {
		let delivered_messages =
			DeliveredMessages { begin: 100, end: 150, dispatch_results: bitvec![u8, Msb0; 1; 151] };

		assert!(!delivered_messages.contains_message(99));
		assert!(delivered_messages.contains_message(100));
		assert!(delivered_messages.contains_message(150));
		assert!(!delivered_messages.contains_message(151));

		assert!(delivered_messages.message_dispatch_result(125));
	}
}
