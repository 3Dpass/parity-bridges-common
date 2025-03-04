// Copyright 2020-2021 Parity Technologies (UK) Ltd.
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

use cumulus_primitives_core::ParaId;
use rialto_parachain_runtime::{AccountId, AuraId, BridgeMillauMessagesConfig, Signature};
use sc_chain_spec::{ChainSpecExtension, ChainSpecGroup};
use sc_service::ChainType;
use serde::{Deserialize, Serialize};
use sp_core::{sr25519, Pair, Public};
use sp_runtime::traits::{IdentifyAccount, Verify};

/// "Names" of the authorities accounts at local testnet.
const LOCAL_AUTHORITIES_ACCOUNTS: [&str; 2] = ["Alice", "Bob"];
/// "Names" of the authorities accounts at development testnet.
const DEV_AUTHORITIES_ACCOUNTS: [&str; 2] = LOCAL_AUTHORITIES_ACCOUNTS;
/// "Names" of all possible authorities accounts.
const ALL_AUTHORITIES_ACCOUNTS: [&str; 2] = LOCAL_AUTHORITIES_ACCOUNTS;
/// "Name" of the `sudo` account.
const SUDO_ACCOUNT: &str = "Sudo";
/// "Name" of the account, which owns the with-Millau messages pallet.
const MILLAU_MESSAGES_PALLET_OWNER: &str = "Millau.MessagesOwner";

/// Specialized `ChainSpec` for the normal parachain runtime.
pub type ChainSpec =
	sc_service::GenericChainSpec<rialto_parachain_runtime::GenesisConfig, Extensions>;

/// Helper function to generate a crypto pair from seed
pub fn get_from_seed<TPublic: Public>(seed: &str) -> <TPublic::Pair as Pair>::Public {
	TPublic::Pair::from_string(&format!("//{}", seed), None)
		.expect("static values are valid; qed")
		.public()
}

/// The extensions for the [`ChainSpec`].
#[derive(
	Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ChainSpecGroup, ChainSpecExtension,
)]
#[serde(deny_unknown_fields)]
pub struct Extensions {
	/// The relay chain of the Parachain.
	pub relay_chain: String,
	/// The id of the Parachain.
	pub para_id: u32,
}

impl Extensions {
	/// Try to get the extension from the given `ChainSpec`.
	pub fn try_get(chain_spec: &dyn sc_service::ChainSpec) -> Option<&Self> {
		sc_chain_spec::get_extension(chain_spec.extensions())
	}
}

type AccountPublic = <Signature as Verify>::Signer;

/// Helper function to generate an account ID from seed
pub fn get_account_id_from_seed<TPublic: Public>(seed: &str) -> AccountId
where
	AccountPublic: From<<TPublic::Pair as Pair>::Public>,
{
	AccountPublic::from(get_from_seed::<TPublic>(seed)).into_account()
}

/// We're using the same set of endowed accounts on all RialtoParachain chains (dev/local) to make
/// sure that all accounts, required for bridge to be functional (e.g. relayers fund account,
/// accounts used by relayers in our test deployments, accounts used for demonstration
/// purposes), are all available on these chains.
fn endowed_accounts() -> Vec<AccountId> {
	let all_authorities = ALL_AUTHORITIES_ACCOUNTS.iter().flat_map(|x| {
		[
			get_account_id_from_seed::<sr25519::Public>(x),
			get_account_id_from_seed::<sr25519::Public>(&format!("{}//stash", x)),
		]
	});
	vec![
		// Sudo account
		get_account_id_from_seed::<sr25519::Public>(SUDO_ACCOUNT),
		// Regular (unused) accounts
		get_account_id_from_seed::<sr25519::Public>("Charlie"),
		get_account_id_from_seed::<sr25519::Public>("Dave"),
		get_account_id_from_seed::<sr25519::Public>("Eve"),
		get_account_id_from_seed::<sr25519::Public>("Ferdie"),
		get_account_id_from_seed::<sr25519::Public>("Charlie//stash"),
		get_account_id_from_seed::<sr25519::Public>("Dave//stash"),
		get_account_id_from_seed::<sr25519::Public>("Eve//stash"),
		get_account_id_from_seed::<sr25519::Public>("Ferdie//stash"),
		// Accounts, used by RialtoParachain<>Millau bridge
		get_account_id_from_seed::<sr25519::Public>(MILLAU_MESSAGES_PALLET_OWNER),
		get_account_id_from_seed::<sr25519::Public>("Millau.HeadersAndMessagesRelay1"),
		get_account_id_from_seed::<sr25519::Public>("Millau.HeadersAndMessagesRelay2"),
		get_account_id_from_seed::<sr25519::Public>("Millau.MessagesSender"),
	]
	.into_iter()
	.chain(all_authorities)
	.collect()
}

pub fn development_config(id: ParaId) -> ChainSpec {
	// Give your base currency a unit name and decimal places
	let mut properties = sc_chain_spec::Properties::new();
	properties.insert("tokenSymbol".into(), "UNIT".into());
	properties.insert("tokenDecimals".into(), 12.into());

	ChainSpec::from_genesis(
		// Name
		"Development",
		// ID
		"dev",
		ChainType::Local,
		move || {
			testnet_genesis(
				get_account_id_from_seed::<sr25519::Public>(SUDO_ACCOUNT),
				DEV_AUTHORITIES_ACCOUNTS.into_iter().map(get_from_seed::<AuraId>).collect(),
				endowed_accounts(),
				id,
			)
		},
		vec![],
		None,
		None,
		None,
		None,
		Extensions {
			relay_chain: "rococo-local".into(), // You MUST set this to the correct network!
			para_id: id.into(),
		},
	)
}

pub fn local_testnet_config(id: ParaId) -> ChainSpec {
	// Give your base currency a unit name and decimal places
	let mut properties = sc_chain_spec::Properties::new();
	properties.insert("tokenSymbol".into(), "UNIT".into());
	properties.insert("tokenDecimals".into(), 12.into());

	ChainSpec::from_genesis(
		// Name
		"Local Testnet",
		// ID
		"local_testnet",
		ChainType::Local,
		move || {
			testnet_genesis(
				get_account_id_from_seed::<sr25519::Public>(SUDO_ACCOUNT),
				LOCAL_AUTHORITIES_ACCOUNTS.into_iter().map(get_from_seed::<AuraId>).collect(),
				endowed_accounts(),
				id,
			)
		},
		Vec::new(),
		None,
		None,
		None,
		None,
		Extensions {
			relay_chain: "rococo-local".into(), // You MUST set this to the correct network!
			para_id: id.into(),
		},
	)
}

fn testnet_genesis(
	root_key: AccountId,
	initial_authorities: Vec<AuraId>,
	endowed_accounts: Vec<AccountId>,
	id: ParaId,
) -> rialto_parachain_runtime::GenesisConfig {
	rialto_parachain_runtime::GenesisConfig {
		system: rialto_parachain_runtime::SystemConfig {
			code: rialto_parachain_runtime::WASM_BINARY
				.expect("WASM binary was not build, please build it!")
				.to_vec(),
		},
		balances: rialto_parachain_runtime::BalancesConfig {
			balances: endowed_accounts.iter().cloned().map(|k| (k, 1 << 60)).collect(),
		},
		sudo: rialto_parachain_runtime::SudoConfig { key: Some(root_key) },
		parachain_info: rialto_parachain_runtime::ParachainInfoConfig { parachain_id: id },
		aura: rialto_parachain_runtime::AuraConfig { authorities: initial_authorities },
		aura_ext: Default::default(),
		bridge_millau_messages: BridgeMillauMessagesConfig {
			owner: Some(get_account_id_from_seed::<sr25519::Public>(MILLAU_MESSAGES_PALLET_OWNER)),
			..Default::default()
		},
	}
}
