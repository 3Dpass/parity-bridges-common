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

//! Primitives of polkadot-like chains, that are related to parachains functionality.
//!
//! Even though this (bridges) repository references polkadot repository, we can't
//! reference polkadot crates from pallets. That's because bridges repository is
//! included in the polkadot repository and included pallets are used by polkadot
//! chains. Having pallets that are referencing polkadot, would mean that there may
//! be two versions of polkadot crates included in the runtime. Which is bad.

use bp_runtime::Size;
use codec::{CompactAs, Decode, Encode, MaxEncodedLen};
use frame_support::RuntimeDebug;
use scale_info::TypeInfo;
use sp_core::Hasher;
use sp_std::vec::Vec;

#[cfg(feature = "std")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "std")]
use parity_util_mem::MallocSizeOf;

/// Parachain id.
///
/// This is an equivalent of the `polkadot_parachain::Id`, which is a compact-encoded `u32`.
#[derive(
	Clone,
	CompactAs,
	Copy,
	Decode,
	Default,
	Encode,
	Eq,
	Hash,
	MaxEncodedLen,
	Ord,
	PartialEq,
	PartialOrd,
	RuntimeDebug,
	TypeInfo,
)]
pub struct ParaId(pub u32);

impl From<u32> for ParaId {
	fn from(id: u32) -> Self {
		ParaId(id)
	}
}

/// Parachain head.
///
/// This is an equivalent of the `polkadot_parachain::HeadData`.
///
/// The parachain head means (at least in Cumulus) a SCALE-encoded parachain header. Keep in mind
/// that in Polkadot it is twice-encoded (so `header.encode().encode()`). We'll also do it to keep
/// it binary-compatible (implies hash-compatibility) with other parachain pallets.
#[derive(
	PartialEq, Eq, Clone, PartialOrd, Ord, Encode, Decode, RuntimeDebug, TypeInfo, Default,
)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize, Hash, MallocSizeOf))]
pub struct ParaHead(pub Vec<u8>);

impl ParaHead {
	/// Returns the hash of this head data.
	pub fn hash(&self) -> crate::Hash {
		sp_runtime::traits::BlakeTwo256::hash(&self.0)
	}
}

/// Parachain head hash.
pub type ParaHash = crate::Hash;

/// Parachain head hasher.
pub type ParaHasher = crate::Hasher;

/// Raw storage proof of parachain heads, stored in polkadot-like chain runtime.
#[derive(Clone, Decode, Encode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
pub struct ParaHeadsProof(pub Vec<Vec<u8>>);

impl Size for ParaHeadsProof {
	fn size(&self) -> u32 {
		u32::try_from(self.0.iter().fold(0usize, |sum, node| sum.saturating_add(node.len())))
			.unwrap_or(u32::MAX)
	}
}
