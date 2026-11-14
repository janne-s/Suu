# Suu's patch to wry 0.35.2

wry 0.35.2 from crates.io (MIT or Apache-2.0, licences alongside), changed in one place:
`src/wkwebview/file_drop.rs`, `collect_paths`. A drag whose pasteboard names the file names type
but holds no array (Ableton Live's own drag of a device over the plug-in window) dereferenced
null and crashed the host. The patched function reads only an array of strings, and otherwise
reports no paths. Later wry versions check this; the plug-in's webview crate pins 0.35.
