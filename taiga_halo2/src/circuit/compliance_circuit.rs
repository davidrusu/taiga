use crate::circuit::blake2s::{vp_commitment_gadget, Blake2sChip, Blake2sConfig};
use crate::circuit::gadgets::assign_free_advice;
use crate::circuit::hash_to_curve::HashToCurveConfig;
use crate::circuit::integrity::{
    check_input_resource, check_output_resource, compute_delta_commitment,
};
use crate::circuit::merkle_circuit::{
    merkle_poseidon_gadget, MerklePoseidonChip, MerklePoseidonConfig,
};
use crate::constant::{
    TaigaFixedBases, COMPLIANCE_ANCHOR_PUBLIC_INPUT_ROW_IDX,
    COMPLIANCE_DELTA_CM_X_PUBLIC_INPUT_ROW_IDX, COMPLIANCE_DELTA_CM_Y_PUBLIC_INPUT_ROW_IDX,
    COMPLIANCE_INPUT_VP_CM_1_ROW_IDX, COMPLIANCE_INPUT_VP_CM_2_ROW_IDX,
    COMPLIANCE_NF_PUBLIC_INPUT_ROW_IDX, COMPLIANCE_OUTPUT_CM_PUBLIC_INPUT_ROW_IDX,
    COMPLIANCE_OUTPUT_VP_CM_1_ROW_IDX, COMPLIANCE_OUTPUT_VP_CM_2_ROW_IDX,
    TAIGA_COMMITMENT_TREE_DEPTH, VP_COMMITMENT_PERSONALIZATION_TO_FIELD,
};
use crate::merkle_tree::LR;
use crate::resource::Resource;

use halo2_gadgets::{
    ecc::chip::{EccChip, EccConfig},
    poseidon::{primitives as poseidon, Pow5Chip as PoseidonChip, Pow5Config as PoseidonConfig},
    utilities::lookup_range_check::LookupRangeCheckConfig,
};
use halo2_proofs::{
    circuit::{floor_planner, Layouter, Value},
    plonk::{
        Advice, Circuit, Column, ConstraintSystem, Constraints, Error, Expression, Instance,
        Selector, TableColumn,
    },
    poly::Rotation,
};
use pasta_curves::pallas;

use crate::circuit::resource_commitment::{ResourceCommitChip, ResourceCommitConfig};

use super::gadgets::poseidon_hash::poseidon_hash_gadget;

#[derive(Clone, Debug)]
pub struct ComplianceConfig {
    instances: Column<Instance>,
    advices: [Column<Advice>; 10],
    table_idx: TableColumn,
    ecc_config: EccConfig<TaigaFixedBases>,
    poseidon_config: PoseidonConfig<pallas::Base, 3, 2>,
    merkle_config: MerklePoseidonConfig,
    merkle_path_selector: Selector,
    hash_to_curve_config: HashToCurveConfig,
    // blake2s_config: Blake2sConfig<pallas::Base>,
    resource_commit_config: ResourceCommitConfig,
}

/// The Compliance circuit.
#[derive(Clone, Debug, Default)]
pub struct ComplianceCircuit {
    /// Input resource
    pub input_resource: Resource,
    /// The authorization path of input resource
    pub merkle_path: [(pallas::Base, LR); TAIGA_COMMITMENT_TREE_DEPTH],
    /// Output resource
    pub output_resource: Resource,
    /// random scalar for delta commitment
    pub rcv: pallas::Scalar,
    /// The randomness for input resource application vp commitment
    pub input_vp_cm_r: pallas::Base,
    /// The randomness for output resource application vp commitment
    pub output_vp_cm_r: pallas::Base,
}

impl Circuit<pallas::Base> for ComplianceCircuit {
    type Config = ComplianceConfig;
    type FloorPlanner = floor_planner::V1;

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    fn configure(meta: &mut ConstraintSystem<pallas::Base>) -> Self::Config {
        let instances = meta.instance_column();
        meta.enable_equality(instances);

        let advices = [
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
        ];

        for advice in advices.iter() {
            meta.enable_equality(*advice);
        }

        let table_idx = meta.lookup_table_column();

        let range_check = LookupRangeCheckConfig::configure(meta, advices[9], table_idx);

        let lagrange_coeffs = [
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
        ];
        meta.enable_constant(lagrange_coeffs[0]);

        let ecc_config =
            EccChip::<TaigaFixedBases>::configure(meta, advices, lagrange_coeffs, range_check);

        let poseidon_config = PoseidonChip::configure::<poseidon::P128Pow5T3>(
            meta,
            advices[6..9].try_into().unwrap(),
            advices[5],
            lagrange_coeffs[2..5].try_into().unwrap(),
            lagrange_coeffs[5..8].try_into().unwrap(),
        );

        let merkle_path_selector = meta.selector();
        meta.create_gate("merkle path check", |meta| {
            let merkle_path_selector = meta.query_selector(merkle_path_selector);
            let is_ephemeral_input = meta.query_advice(advices[0], Rotation::cur());
            let anchor = meta.query_advice(advices[1], Rotation::cur());
            let root = meta.query_advice(advices[2], Rotation::cur());
            let constant_one = Expression::Constant(pallas::Base::one());

            Constraints::with_selector(
                merkle_path_selector,
                [(
                    "is_ephemeral is true, or root = anchor",
                    (constant_one - is_ephemeral_input) * (root - anchor),
                )],
            )
        });

        let merkle_config = MerklePoseidonChip::configure(
            meta,
            advices[..5].try_into().unwrap(),
            poseidon_config.clone(),
        );

        let hash_to_curve_config =
            HashToCurveConfig::configure(meta, advices, poseidon_config.clone());

        // let blake2s_config = Blake2sConfig::configure(meta, advices);

        let resource_commit_config = ResourceCommitChip::configure(
            meta,
            advices[0..3].try_into().unwrap(),
            poseidon_config.clone(),
            range_check,
        );

        Self::Config {
            instances,
            advices,
            table_idx,
            ecc_config,
            poseidon_config,
            merkle_config,
            merkle_path_selector,
            hash_to_curve_config,
            // blake2s_config,
            resource_commit_config,
        }
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<pallas::Base>,
    ) -> Result<(), Error> {
        // Construct an ECC chip
        let ecc_chip = EccChip::construct(config.ecc_config);
        layouter.assign_table(
            || "table_idx",
            |mut table| {
                // We generate the row values lazily (we only need them during keygen).
                for index in 0..(1 << 10) {
                    table.assign_cell(
                        || "table_idx",
                        config.table_idx,
                        index,
                        || Value::known(pallas::Base::from(index as u64)),
                    )?;
                }
                Ok(())
            },
        )?;

        // Construct a merkle chip
        let merkle_chip = MerklePoseidonChip::construct(config.merkle_config);

        // Construct a blake2s chip
        // let blake2s_chip = Blake2sChip::construct(config.blake2s_config);

        // Construct a resource_commit chip
        let resource_commit_chip = ResourceCommitChip::construct(config.resource_commit_config);

        // Input resource
        // Check the input resource commitment
        let input_resource_variables = check_input_resource(
            layouter.namespace(|| "check input resource"),
            config.advices,
            config.instances,
            resource_commit_chip.clone(),
            self.input_resource,
            COMPLIANCE_NF_PUBLIC_INPUT_ROW_IDX,
        )?;

        // Check the merkle tree path validity and public the root
        let root = merkle_poseidon_gadget(
            layouter.namespace(|| "poseidon merkle"),
            merkle_chip,
            input_resource_variables.cm,
            &self.merkle_path,
        )?;

        // Output resource
        let output_resource_vars = check_output_resource(
            layouter.namespace(|| "check output resource"),
            config.advices,
            config.instances,
            resource_commit_chip,
            self.output_resource,
            input_resource_variables.nf,
            COMPLIANCE_OUTPUT_CM_PUBLIC_INPUT_ROW_IDX,
        )?;

        // compute and public delta commitment(input_value_commitment - output_value_commitment)
        let delta = compute_delta_commitment(
            layouter.namespace(|| "delta commitment"),
            ecc_chip,
            config.hash_to_curve_config.clone(),
            input_resource_variables.resource_variables.logic.clone(),
            input_resource_variables.resource_variables.label.clone(),
            input_resource_variables.resource_variables.quantity.clone(),
            output_resource_vars.resource_variables.logic.clone(),
            output_resource_vars.resource_variables.label.clone(),
            output_resource_vars.resource_variables.quantity,
            self.rcv,
        )?;
        layouter.constrain_instance(
            delta.inner().x().cell(),
            config.instances,
            COMPLIANCE_DELTA_CM_X_PUBLIC_INPUT_ROW_IDX,
        )?;
        layouter.constrain_instance(
            delta.inner().y().cell(),
            config.instances,
            COMPLIANCE_DELTA_CM_Y_PUBLIC_INPUT_ROW_IDX,
        )?;

        // merkle path check
        layouter.assign_region(
            || "merkle path check",
            |mut region| {
                input_resource_variables
                    .resource_variables
                    .is_ephemeral
                    .copy_advice(|| "is_ephemeral_input", &mut region, config.advices[0], 0)?;
                region.assign_advice_from_instance(
                    || "anchor",
                    config.instances,
                    COMPLIANCE_ANCHOR_PUBLIC_INPUT_ROW_IDX,
                    config.advices[1],
                    0,
                )?;
                root.copy_advice(|| "root", &mut region, config.advices[2], 0)?;
                config.merkle_path_selector.enable(&mut region, 0)
            },
        )?;

        // Input resource application VP commitment
        let input_vp_cm_r = assign_free_advice(
            layouter.namespace(|| "witness input_vp_cm_r"),
            config.advices[0],
            Value::known(self.input_vp_cm_r),
        )?;

        // let vp_commitment_personalization = assign_free_advice(
        //     layouter.namespace(|| "constant VP_COMMITMENT_PERSONALIZATION_TO_FIELD"),
        //     config.advices[0],
        //     Value::known(*VP_COMMITMENT_PERSONALIZATION_TO_FIELD),
        // )?;

        let input_vp_commitment = poseidon_hash_gadget(
            config.poseidon_config.clone(),
            layouter.namespace(|| "input vp commitment"),
            [
                // vp_commitment_personalization.clone(),
                input_resource_variables.resource_variables.logic.clone(),
                input_vp_cm_r,
            ],
        )?;

        // let input_vp_commitment = vp_commitment_gadget(
        //     &mut layouter,
        //     &blake2s_chip,
        //     input_resource_variables.resource_variables.logic.clone(),
        //     input_vp_cm_r,
        // )?;
        // layouter.constrain_instance(
        //     input_vp_commitment.cell(),
        //     config.instances,
        //     COMPLIANCE_INPUT_VP_CM_1_ROW_IDX,
        // )?;
        // layouter.constrain_instance(
        //     input_vp_commitment.cell(),
        //     config.instances,
        //     COMPLIANCE_INPUT_VP_CM_2_ROW_IDX,
        // )?;

        // Output resource application VP commitment
        let output_vp_cm_r = assign_free_advice(
            layouter.namespace(|| "witness output_vp_cm_r"),
            config.advices[0],
            Value::known(self.output_vp_cm_r),
        )?;

        let output_vp_commitment = poseidon_hash_gadget(
            config.poseidon_config,
            layouter.namespace(|| "output vp commitment"),
            [
                // vp_commitment_personalization,
                output_resource_vars.resource_variables.logic.clone(),
                output_vp_cm_r,
            ],
        )?;

        // let output_vp_commitment = vp_commitment_gadget(
        //     &mut layouter,
        //     &blake2s_chip,
        //     output_resource_vars.resource_variables.logic.clone(),
        //     output_vp_cm_r,
        // )?;
        // layouter.constrain_instance(
        //     output_vp_commitment.cell(),
        //     config.instances,
        //     COMPLIANCE_OUTPUT_VP_CM_1_ROW_IDX,
        // )?;
        // layouter.constrain_instance(
        //     output_vp_commitment.cell(),
        //     config.instances,
        //     COMPLIANCE_OUTPUT_VP_CM_2_ROW_IDX,
        // )?;

        Ok(())
    }
}

#[test]
fn test_halo2_compliance_circuit() {
    use crate::compliance::tests::random_compliance_info;
    use crate::constant::{
        COMPLIANCE_CIRCUIT_PARAMS_SIZE, COMPLIANCE_PROVING_KEY, COMPLIANCE_VERIFYING_KEY,
        SETUP_PARAMS_MAP,
    };
    use crate::proof::Proof;
    use halo2_proofs::dev::MockProver;

    use rand::rngs::OsRng;

    let mut rng = OsRng;
    let compliance_info = random_compliance_info(&mut rng);
    let (compliance, compliance_circuit) = compliance_info.build();
    let instances = vec![compliance.to_instance()];
    let prover = MockProver::<pallas::Base>::run(
        COMPLIANCE_CIRCUIT_PARAMS_SIZE,
        &compliance_circuit,
        instances,
    )
    .unwrap();
    assert_eq!(prover.verify(), Ok(()));

    // Create compliance proof
    let params = SETUP_PARAMS_MAP
        .get(&COMPLIANCE_CIRCUIT_PARAMS_SIZE)
        .unwrap();
    let proof = Proof::create(
        &COMPLIANCE_PROVING_KEY,
        params,
        compliance_circuit,
        &[&compliance.to_instance()],
        &mut rng,
    )
    .unwrap();

    assert!(proof
        .verify(
            &COMPLIANCE_VERIFYING_KEY,
            params,
            &[&compliance.to_instance()]
        )
        .is_ok());
}
