fn main() {
    uniffi::generate_scaffolding("src/phonecam.udl")
        .expect("failed to generate UniFFI scaffolding");
}
