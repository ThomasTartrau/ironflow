use std::fs;
use std::path::Path;

use progenitor::GenerationSettings;
use progenitor::Generator;
use progenitor::InterfaceStyle;
use progenitor::TagStyle;

fn main() {
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/openapi.json");
    // Build scripts emit cargo directives via stdout -- tracing is not available here
    #[allow(clippy::print_stdout)]
    {
        std::io::Write::write_all(
            &mut std::io::stdout(),
            format!("cargo:rerun-if-changed={src}\n").as_bytes(),
        )
        .expect("failed to write cargo directive");
    }

    let text = fs::read_to_string(src).expect("failed to read openapi.json");
    let spec = serde_json::from_str(&text).expect("failed to parse openapi.json");

    let mut settings = GenerationSettings::default();
    settings.with_interface(InterfaceStyle::Builder);
    settings.with_tag(TagStyle::Merged);

    let mut generator = Generator::new(&settings);

    let tokens = generator
        .generate_tokens(&spec)
        .expect("failed to generate SDK");
    let ast = syn::parse2(tokens).expect("failed to parse generated tokens");
    let content = prettyplease::unparse(&ast);

    let content = content
        .replace("/// ```\n", "/// ```ignore\n")
        .replace("/// ```rust\n", "/// ```ignore\n");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_file = Path::new(&out_dir).join("codegen.rs");

    fs::write(out_file, content).expect("failed to write generated code");
}
