// Copyright 2019-2022 Parity Technologies (UK) Ltd.
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

use async_trait::async_trait;
use std::sync::Arc;

use crate::cli::{
	bridge::{
		CliBridgeBase, MessagesCliBridge, ParachainToRelayHeadersCliBridge,
		RelayToRelayHeadersCliBridge,
	},
	relay_headers_and_messages::{Full2WayBridgeBase, Full2WayBridgeCommonParams},
	CliChain,
};
use bp_polkadot_core::parachains::ParaHash;
use bp_runtime::BlockNumberOf;
use pallet_bridge_parachains::{RelayBlockHash, RelayBlockHasher, RelayBlockNumber};
use relay_substrate_client::{AccountIdOf, AccountKeyPairOf, Chain, Client, TransactionSignScheme};
use sp_core::Pair;
use substrate_relay_helper::{
	finality::SubstrateFinalitySyncPipeline,
	on_demand::{
		headers::OnDemandHeadersRelay, parachains::OnDemandParachainsRelay, OnDemandRelay,
	},
	TaggedAccount, TransactionParams,
};

pub struct RelayToParachainBridge<
	L2R: MessagesCliBridge + RelayToRelayHeadersCliBridge,
	R2L: MessagesCliBridge + ParachainToRelayHeadersCliBridge,
> {
	pub common:
		Full2WayBridgeCommonParams<<R2L as CliBridgeBase>::Target, <L2R as CliBridgeBase>::Target>,
	pub right_relay: Client<<R2L as ParachainToRelayHeadersCliBridge>::SourceRelay>,

	// override for right_relay->left headers signer
	pub right_headers_to_left_transaction_params:
		TransactionParams<AccountKeyPairOf<<R2L as CliBridgeBase>::Target>>,
	// override for right->left parachains signer
	pub right_parachains_to_left_transaction_params:
		TransactionParams<AccountKeyPairOf<<R2L as CliBridgeBase>::Target>>,
	// override for left->right headers signer
	pub left_headers_to_right_transaction_params:
		TransactionParams<AccountKeyPairOf<<L2R as CliBridgeBase>::Target>>,
}

macro_rules! declare_relay_to_parachain_bridge_schema {
	// chain, parachain, relay-chain-of-parachain
	($left_chain:ident, $right_parachain:ident, $right_chain:ident) => {
		bp_runtime::paste::item! {
			#[doc = $left_chain ", " $right_parachain " and " $right_chain " headers+parachains+messages relay params."]
			#[derive(Debug, PartialEq, StructOpt)]
			pub struct [<$left_chain $right_parachain HeadersAndMessages>] {
				#[structopt(flatten)]
				shared: HeadersAndMessagesSharedParams,
				#[structopt(flatten)]
				left: [<$left_chain ConnectionParams>],
				// default signer, which is always used to sign messages relay transactions on the left chain
				#[structopt(flatten)]
				left_sign: [<$left_chain SigningParams>],
				// override for right_relay->left headers signer
				#[structopt(flatten)]
				right_relay_headers_to_left_sign_override: [<$right_chain HeadersTo $left_chain SigningParams>],
				// override for right->left parachains signer
				#[structopt(flatten)]
				right_parachains_to_left_sign_override: [<$right_chain ParachainsTo $left_chain SigningParams>],
				#[structopt(flatten)]
				left_messages_pallet_owner: [<$left_chain MessagesPalletOwnerSigningParams>],
				#[structopt(flatten)]
				right: [<$right_parachain ConnectionParams>],
				// default signer, which is always used to sign messages relay transactions on the right chain
				#[structopt(flatten)]
				right_sign: [<$right_parachain SigningParams>],
				// override for left->right headers signer
				#[structopt(flatten)]
				left_headers_to_right_sign_override: [<$left_chain HeadersTo $right_parachain SigningParams>],
				#[structopt(flatten)]
				right_messages_pallet_owner: [<$right_parachain MessagesPalletOwnerSigningParams>],
				#[structopt(flatten)]
				right_relay: [<$right_chain ConnectionParams>],
			}

			impl [<$left_chain $right_parachain HeadersAndMessages>] {
				async fn into_bridge<
					Left: TransactionSignScheme + CliChain<KeyPair = AccountKeyPairOf<Left>>,
					Right: TransactionSignScheme + CliChain<KeyPair = AccountKeyPairOf<Right>>,
					RightRelay: TransactionSignScheme + CliChain,
					L2R: CliBridgeBase<Source = Left, Target = Right> + MessagesCliBridge + RelayToRelayHeadersCliBridge,
					R2L: CliBridgeBase<Source = Right, Target = Left>
						+ MessagesCliBridge
						+ ParachainToRelayHeadersCliBridge<SourceRelay = RightRelay>,
				>(
					self,
				) -> anyhow::Result<RelayToParachainBridge<L2R, R2L>> {
					Ok(RelayToParachainBridge {
						common: Full2WayBridgeCommonParams::new::<L2R>(
							self.shared,
							BridgeEndCommonParams {
								client: self.left.into_client::<Left>().await?,
								sign: self.left_sign.to_keypair::<Left>()?,
								transactions_mortality: self.left_sign.transactions_mortality()?,
								messages_pallet_owner: self.left_messages_pallet_owner.to_keypair::<Left>()?,
								accounts: vec![],
							},
							BridgeEndCommonParams {
								client: self.right.into_client::<Right>().await?,
								sign: self.right_sign.to_keypair::<Right>()?,
								transactions_mortality: self.right_sign.transactions_mortality()?,
								messages_pallet_owner: self.right_messages_pallet_owner.to_keypair::<Right>()?,
								accounts: vec![],
							},
						)?,
						right_relay: self.right_relay.into_client::<RightRelay>().await?,
						right_headers_to_left_transaction_params: self
							.right_relay_headers_to_left_sign_override
							.transaction_params_or::<Left, _>(
							&self.left_sign,
						)?,
						right_parachains_to_left_transaction_params: self
							.right_parachains_to_left_sign_override
							.transaction_params_or::<Left, _>(
							&self.left_sign,
						)?,
						left_headers_to_right_transaction_params: self
							.left_headers_to_right_sign_override
							.transaction_params_or::<Right, _>(&self.right_sign)?,
					})
				}
			}
		}
	};
}

#[async_trait]
impl<
		Left: Chain + TransactionSignScheme<Chain = Left> + CliChain<KeyPair = AccountKeyPairOf<Left>>,
		Right: Chain<Hash = ParaHash>
			+ TransactionSignScheme<Chain = Right>
			+ CliChain<KeyPair = AccountKeyPairOf<Right>>,
		RightRelay: Chain<BlockNumber = RelayBlockNumber, Hash = RelayBlockHash, Hasher = RelayBlockHasher>
			+ TransactionSignScheme
			+ CliChain,
		L2R: CliBridgeBase<Source = Left, Target = Right>
			+ MessagesCliBridge
			+ RelayToRelayHeadersCliBridge,
		R2L: CliBridgeBase<Source = Right, Target = Left>
			+ MessagesCliBridge
			+ ParachainToRelayHeadersCliBridge<SourceRelay = RightRelay>,
	> Full2WayBridgeBase for RelayToParachainBridge<L2R, R2L>
where
	AccountIdOf<Left>: From<<AccountKeyPairOf<Left> as Pair>::Public>,
	AccountIdOf<Right>: From<<AccountKeyPairOf<Right> as Pair>::Public>,
{
	type Params = RelayToParachainBridge<L2R, R2L>;
	type Left = Left;
	type Right = Right;

	fn common(&self) -> &Full2WayBridgeCommonParams<Left, Right> {
		&self.common
	}

	fn mut_common(&mut self) -> &mut Full2WayBridgeCommonParams<Self::Left, Self::Right> {
		&mut self.common
	}

	async fn start_on_demand_headers_relayers(
		&mut self,
	) -> anyhow::Result<(
		Arc<dyn OnDemandRelay<BlockNumberOf<Self::Left>>>,
		Arc<dyn OnDemandRelay<BlockNumberOf<Self::Right>>>,
	)> {
		self.common.left.accounts.push(TaggedAccount::Headers {
			id: self.right_headers_to_left_transaction_params.signer.public().into(),
			bridged_chain: RightRelay::NAME.to_string(),
		});
		self.common.left.accounts.push(TaggedAccount::Parachains {
			id: self.right_parachains_to_left_transaction_params.signer.public().into(),
			bridged_chain: RightRelay::NAME.to_string(),
		});
		self.common.right.accounts.push(TaggedAccount::Headers {
			id: self.left_headers_to_right_transaction_params.signer.public().into(),
			bridged_chain: Left::NAME.to_string(),
		});

		<L2R as RelayToRelayHeadersCliBridge>::Finality::start_relay_guards(
			&self.common.right.client,
			&self.left_headers_to_right_transaction_params,
			self.common.right.client.can_start_version_guard(),
		)
		.await?;
		<R2L as ParachainToRelayHeadersCliBridge>::RelayFinality::start_relay_guards(
			&self.common.left.client,
			&self.right_headers_to_left_transaction_params,
			self.common.left.client.can_start_version_guard(),
		)
		.await?;

		let left_to_right_on_demand_headers =
			OnDemandHeadersRelay::new::<<L2R as RelayToRelayHeadersCliBridge>::Finality>(
				self.common.left.client.clone(),
				self.common.right.client.clone(),
				self.left_headers_to_right_transaction_params.clone(),
				self.common.shared.only_mandatory_headers,
			);
		let right_relay_to_left_on_demand_headers =
			OnDemandHeadersRelay::new::<<R2L as ParachainToRelayHeadersCliBridge>::RelayFinality>(
				self.right_relay.clone(),
				self.common.left.client.clone(),
				self.right_headers_to_left_transaction_params.clone(),
				self.common.shared.only_mandatory_headers,
			);
		let right_to_left_on_demand_parachains = OnDemandParachainsRelay::new::<
			<R2L as ParachainToRelayHeadersCliBridge>::ParachainFinality,
		>(
			self.right_relay.clone(),
			self.common.left.client.clone(),
			self.right_parachains_to_left_transaction_params.clone(),
			Arc::new(right_relay_to_left_on_demand_headers),
		);

		Ok((
			Arc::new(left_to_right_on_demand_headers),
			Arc::new(right_to_left_on_demand_parachains),
		))
	}
}
