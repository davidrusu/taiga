use taiga_halo2::circuit::compliance_circuit::ComplianceCircuit;

fn main() {
    let circuit = ComplianceCircuit::default();
    let dot_string = halo2_proofs::dev::circuit_dot_graph(&circuit);
    print!("{}", dot_string);

    // Now you can either handle it in Rust, or just
    // print it out to use with command-line tools.

    // // Create the area you want to draw on.
    // // Use SVGBackend if you want to render to .svg instead.
    use plotters::prelude::*;
    let root = SVGBackend::new("compliance.svg", (1024 * 20, 768 * 20)).into_drawing_area();
    root.fill(&WHITE).unwrap();
    // let root = root
    //     .titled("Compliance Circuit Layout", ("monospace", 60))
    //     .unwrap();

    halo2_proofs::dev::CircuitLayout::default()
        // You can optionally render only a section of the circuit.
        // .view_width(0..2)
        // .view_height(0..16)
        // You can hide labels, which can be useful with smaller areas.
        // .show_labels(false)
        // Render the circuit onto your area!
        // The first argument is the size parameter for the circuit.
        .render(15, &circuit, &root)
        .unwrap();
}
