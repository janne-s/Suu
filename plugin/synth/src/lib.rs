//! Suu Synth, the instrument: the internal oscillator through the mouth, a sidechain as input.
use nih_plug::prelude::*;

nih_export_clap!(suu_plugin::SuuSynth);
nih_export_vst3!(suu_plugin::SuuSynth);
