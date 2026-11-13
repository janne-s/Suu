//! Hosts store automation and saved state under the parameter IDs: an ID that changes breaks
//! every project that used it. This test holds them to `param_ids.txt`. Adding a parameter means
//! appending its ID there (SUU_BLESS_IDS=1 writes the file); removing or renaming one fails.

use nih_plug::prelude::Params;
use suu_plugin::params::SuuParams;

fn ids(synth: bool) -> Vec<String> {
    SuuParams::new(synth).param_map().into_iter().map(|(id, _, _)| id).collect()
}

#[test]
fn ids_are_unique_and_kept() {
    let fx = ids(false);
    assert_eq!(fx, ids(true), "the effect and the instrument share one parameter list");
    let mut sorted = fx.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), fx.len(), "duplicate parameter IDs");
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/param_ids.txt");
    let now = fx.join("\n") + "\n";
    if std::env::var_os("SUU_BLESS_IDS").is_some() {
        std::fs::write(path, &now).unwrap();
    }
    let kept = std::fs::read_to_string(path).expect("tests/param_ids.txt");
    let missing: Vec<&str> = kept.lines().filter(|id| !fx.iter().any(|f| f == id)).collect();
    assert!(missing.is_empty(), "parameter IDs removed or renamed: {missing:?}");
    let added: Vec<&String> = fx.iter().filter(|f| !kept.lines().any(|k| k == f.as_str())).collect();
    assert!(added.is_empty(), "new parameter IDs not yet in param_ids.txt (SUU_BLESS_IDS=1): {added:?}");
}
