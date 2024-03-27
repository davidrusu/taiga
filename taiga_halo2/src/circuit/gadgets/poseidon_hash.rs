use halo2_gadgets::poseidon::{
    primitives as poseidon, primitives::ConstantLength, Hash as PoseidonHash,
    Pow5Chip as PoseidonChip, Pow5Config as PoseidonConfig,
};
use halo2_proofs::{
    circuit::{AssignedCell, Layouter},
    plonk::Error,
};
use pasta_curves::pallas;

const WIDTH: usize = 3;
const RATE: usize = 2;

pub fn poseidon_hash_gadget<const L: usize>(
    config: PoseidonConfig<pallas::Base, WIDTH, RATE>,
    mut layouter: impl Layouter<pallas::Base>,
    messages: [AssignedCell<pallas::Base, pallas::Base>; L],
) -> Result<AssignedCell<pallas::Base, pallas::Base>, Error> {
    let poseidon_chip = PoseidonChip::construct(config);
    let poseidon_hasher =
        PoseidonHash::<_, _, poseidon::P128Pow5T3, ConstantLength<L>, WIDTH, RATE>::init(
            poseidon_chip,
            layouter.namespace(|| "Poseidon init"),
        )?;

    poseidon_hasher.hash(layouter.namespace(|| "poseidon hash"), messages)
}
