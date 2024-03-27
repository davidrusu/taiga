use crate::{constant::VP_COMMITMENT_PERSONALIZATION, utils::poseidon_hash_n};
use blake2s_simd::Params;
use byteorder::{ByteOrder, LittleEndian};
use ff::PrimeField;
use pasta_curves::pallas;
#[cfg(feature = "nif")]
use rustler::NifTuple;
#[cfg(feature = "serde")]
use serde;

type Fp = pallas::Base;

#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "nif", derive(NifTuple))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ValidityPredicateCommitment(Fp);

impl ValidityPredicateCommitment {
    pub fn commit(vp: Fp, rcm: Fp) -> Self {
        Self(poseidon_hash_n([
            // VP_COMMITMENT_PERSONALIZATION,
            vp, rcm,
        ]))
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_repr()
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Fp::from_repr(bytes).unwrap())
    }

    pub fn from_public_input(public_input: Fp) -> Self {
        Self(public_input)
    }

    pub fn to_public_input(&self) -> Fp {
        self.0
    }
}
