//! Build-time glue for the official Wasm bindings generator; no installed CLI needed.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = args.next().ok_or("expected input .wasm")?;
    let output = args.next().ok_or("expected output directory")?;
    if args.next().is_some() {
        return Err("expected exactly input and output arguments".into());
    }
    wasm_bindgen_cli_support::Bindgen::new()
        .input_path(input)
        .out_name("ccht")
        .web(true)?
        .typescript(true)
        .generate(output)?;
    Ok(())
}
