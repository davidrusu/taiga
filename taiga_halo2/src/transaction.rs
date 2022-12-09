use crate::action::{ActionInfo, ActionInstance, OutputNoteInfo, SpendNoteInfo};
use crate::circuit::action_circuit::ActionCircuit;
use crate::circuit::vp_circuit::{VPVerifyingInfo, ValidityPredicateInfo};
use crate::constant::NUM_NOTE;
use crate::note::NoteCommitment;
use crate::nullifier::Nullifier;
use crate::value_commitment::ValueCommitment;
use halo2_proofs::{
    plonk::{create_proof, keygen_pk, keygen_vk, verify_proof, Error, SingleVerifier},
    poly::commitment::Params,
    transcript::{Blake2bRead, Blake2bWrite},
};
use pasta_curves::vesta;
use rand::RngCore;

#[derive(Debug, Clone)]
pub struct Transaction {
    partial_txs: Vec<PartialTransaction>,
    // TODO: add binding signature to check sum balance
}

#[derive(Debug, Clone)]
pub struct PartialTransaction {
    actions: [ActionVerifyingInfo; NUM_NOTE],
    spends: [NoteVPVerifyingInfoSet; NUM_NOTE],
    outputs: [NoteVPVerifyingInfoSet; NUM_NOTE],
}

#[derive(Debug, Clone)]
pub struct ActionVerifyingInfo {
    action_proof: Vec<u8>,
    action_instance: ActionInstance,
}

#[derive(Debug, Clone)]
pub struct NoteVPVerifyingInfoSet {
    app_vp_verifying_info: VPVerifyingInfo,
    app_logic_vp_verifying_info: Vec<VPVerifyingInfo>,
    // TODO: add verifier proof and according public inputs.
    // When the verifier proof is added, we may need to reconsider the structure of `VPVerifyingInfo`
}

impl Transaction {
    pub fn get_nullifiers(&self) -> Vec<Nullifier> {
        self.partial_txs
            .iter()
            .flat_map(|ptx| ptx.get_nullifiers())
            .collect()
    }

    pub fn get_output_cms(&self) -> Vec<NoteCommitment> {
        self.partial_txs
            .iter()
            .flat_map(|ptx| ptx.get_output_cms())
            .collect()
    }

    pub fn get_value_commitments(&self) -> Vec<ValueCommitment> {
        self.partial_txs
            .iter()
            .flat_map(|ptx| ptx.get_value_commitments())
            .collect()
    }
}

impl PartialTransaction {
    pub fn build<R: RngCore>(
        spend_info: [SpendNoteInfo; NUM_NOTE],
        output_info: [OutputNoteInfo; NUM_NOTE],
        mut rng: R,
    ) -> Self {
        let spends: Vec<NoteVPVerifyingInfoSet> = spend_info
            .iter()
            .map(|spend_note| {
                NoteVPVerifyingInfoSet::build(
                    spend_note.get_app_vp_proving_info(),
                    spend_note.get_app_logic_vp_proving_info(),
                )
            })
            .collect();
        let outputs: Vec<NoteVPVerifyingInfoSet> = output_info
            .iter()
            .map(|output_note| {
                NoteVPVerifyingInfoSet::build(
                    output_note.get_app_vp_proving_info(),
                    output_note.get_app_logic_vp_proving_info(),
                )
            })
            .collect();
        let actions: Vec<ActionVerifyingInfo> = spend_info
            .into_iter()
            .zip(output_info.into_iter())
            .map(|(spend, output)| {
                let action_info = ActionInfo::new(spend, output);
                ActionVerifyingInfo::create(action_info, &mut rng).unwrap()
            })
            .collect();

        Self {
            actions: actions.try_into().unwrap(),
            spends: spends.try_into().unwrap(),
            outputs: outputs.try_into().unwrap(),
        }
    }

    pub fn verify(&self) -> Result<(), Error> {
        // Verify action proofs
        for verifying_info in self.actions.iter() {
            verifying_info.verify()?;
        }

        // Verify proofs in spend notes
        for verifying_info in self.spends.iter() {
            verifying_info.verify()?;
        }
        // Verify proofs in output notes
        for verifying_info in self.outputs.iter() {
            verifying_info.verify()?;
        }

        Ok(())
    }

    pub fn get_nullifiers(&self) -> Vec<Nullifier> {
        self.actions
            .iter()
            .map(|action| action.action_instance.nf)
            .collect()
    }

    pub fn get_output_cms(&self) -> Vec<NoteCommitment> {
        self.actions
            .iter()
            .map(|action| action.action_instance.cm)
            .collect()
    }

    pub fn get_value_commitments(&self) -> Vec<ValueCommitment> {
        self.actions
            .iter()
            .map(|action| action.action_instance.cv_net)
            .collect()
    }
}

impl ActionVerifyingInfo {
    pub fn create<R: RngCore>(action_info: ActionInfo, mut rng: R) -> Result<Self, Error> {
        let (action_instance, circuit) = action_info.build(&mut rng);
        let params = Params::new(11);
        let empty_circuit: ActionCircuit = Default::default();
        let vk = keygen_vk(&params, &empty_circuit).expect("keygen_vk should not fail");
        let pk = keygen_pk(&params, vk, &empty_circuit).expect("keygen_pk should not fail");
        let mut transcript = Blake2bWrite::<_, vesta::Affine, _>::init(vec![]);
        create_proof(
            &params,
            &pk,
            &[circuit],
            &[&[&action_instance.to_instance()]],
            &mut rng,
            &mut transcript,
        )?;
        let action_proof = transcript.finalize();
        Ok(Self {
            action_proof,
            action_instance,
        })
    }

    pub fn verify(&self) -> Result<(), Error> {
        let params: Params<vesta::Affine> = Params::new(11);
        let empty_circuit: ActionCircuit = Default::default();
        let vk = keygen_vk(&params, &empty_circuit).expect("keygen_vk should not fail");
        let strategy = SingleVerifier::new(&params);
        let mut transcript = Blake2bRead::init(&self.action_proof[..]);
        verify_proof(
            &params,
            &vk,
            strategy,
            &[&[&self.action_instance.to_instance()]],
            &mut transcript,
        )
    }
}

impl NoteVPVerifyingInfoSet {
    pub fn new(
        app_vp_verifying_info: VPVerifyingInfo,
        app_logic_vp_verifying_info: Vec<VPVerifyingInfo>,
    ) -> Self {
        Self {
            app_vp_verifying_info,
            app_logic_vp_verifying_info,
        }
    }

    pub fn build(
        app_vp_proving_info: Box<dyn ValidityPredicateInfo>,
        app_logic_vp_proving_info: Vec<Box<dyn ValidityPredicateInfo>>,
    ) -> Self {
        let app_vp_verifying_info = app_vp_proving_info.get_verifying_info();

        let app_logic_vp_verifying_info = app_logic_vp_proving_info
            .into_iter()
            .map(|proving_info| proving_info.get_verifying_info())
            .collect();

        Self {
            app_vp_verifying_info,
            app_logic_vp_verifying_info,
        }
    }

    pub fn verify(&self) -> Result<(), Error> {
        // Verify application vp proof
        self.app_vp_verifying_info.verify()?;

        // Verify application logic vp proofs
        for verify_info in self.app_logic_vp_verifying_info.iter() {
            verify_info.verify()?;
        }

        // TODO: Verify vp verifier proofs

        Ok(())
    }
}
