# Suu's patch to wry 0.35.2

wry 0.35.2 from crates.io (MIT or Apache-2.0, licences alongside), changed in one place:
`src/wkwebview/file_drop.rs`, `collect_paths`, plus a test (`suu_tests`; run it here with
`cargo test --lib suu_tests`).

Ableton Live's own drag (a device dragged over the plug-in window) names the file names
pasteboard type with an empty list. wry iterated it with cocoa-foundation 0.1.2's
`NSFastIterator`, which reads the enumeration's mutations pointer before it knows whether there
are items; an empty enumeration leaves that pointer null, and the read crashed the host
(2026-11-14, twice). The patched function reads only an array, by `count` and `objectAtIndex:`,
and only its strings. Later wry versions read the list differently; the plug-in's webview crate
pins 0.35.
