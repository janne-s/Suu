//! `cargo xtask bundle -p suu_fx -p suu_synth [--release]`: builds and bundles VST3 and CLAP.
//!
//! nih_plug_xtask's own entry point moves to the topmost directory with a `Cargo.toml`, which
//! here is the repository's workspace, not this one; so this calls its build and bundle steps directly
//! from the plug-in's workspace.
use std::path::Path;

fn main() -> nih_plug_xtask::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask sits in the plug-in workspace");
    std::env::set_current_dir(root)?;
    let mut args = std::env::args().skip(1);
    let usage = "usage: cargo xtask bundle -p <package> [-p <package> …] [--release]";
    match args.next().as_deref() {
        Some("bundle") => {}
        _ => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    }
    let (mut packages, mut rest) = (Vec::new(), Vec::new());
    while let Some(a) = args.next() {
        if a == "-p" || a == "--package" {
            packages.extend(args.next());
        } else {
            rest.push(a);
        }
    }
    if packages.is_empty() {
        eprintln!("{usage}");
        std::process::exit(2);
    }
    nih_plug_xtask::build(&packages, &rest)?;
    let target = root.join("target");
    for p in &packages {
        nih_plug_xtask::bundle(&target, p, &rest, false)?;
    }
    Ok(())
}
