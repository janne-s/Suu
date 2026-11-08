//! Formants and dips of one calibrated vowel with articulator offsets, for tuning a posture
//! against measured singers. Frequency-domain oracle of the airway (no audio).
//!
//! `cargo run --release -q -p chimera --example vowel_tf -- <male|female> <ipa> [key=offset,...] [--fossa-scale s]`
//! Prints F1–F5 (peak frequency, −3 dB bandwidth) and the three deepest dips between 2 and 6 kHz,
//! with and without the piriform fossae. `--fossa-scale` multiplies the fossae's length
//! (rebuilds the airway with a longer or shorter side branch).

use chimera::airway::frequency::{find_peaks, transfer_function};
use chimera::anatomy::human::{ArticKey, BODY_FEMALE, BODY_MALE, NEUTRAL};
use chimera::phonetics::planner::vowel_table;
use chimera::voice::human_airway::HumanAirway;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let body_name = args[0].as_str();
    let ipa = args[1].as_str();
    let offsets: Vec<(ArticKey, f64)> = args
        .get(2)
        .filter(|s| !s.starts_with("--") && !s.is_empty())
        .map(|s| {
            s.split(',')
                .map(|kv| {
                    let (k, v) = kv.split_once('=').expect("key=value");
                    (ArticKey::from_name(k).unwrap_or_else(|| panic!("unknown articulator {k}")), v.parse().expect("number"))
                })
                .collect()
        })
        .unwrap_or_default();
    let body = if body_name == "male" { BODY_MALE } else { BODY_FEMALE };
    let mut air = HumanAirway::new(96000.0, body);
    let mut a = NEUTRAL;
    for k in ArticKey::ALL {
        if let Some(v) = vowel_table()[body_name][ipa].artic.get(k) {
            *a.get_mut(k) = v;
        }
    }
    for (k, d) in &offsets {
        *a.get_mut(*k) += d;
    }
    air.apply(&a, 0.0);
    let el = air.net.linear_elements();
    let freqs: Vec<f64> = (0..3000).map(|i| 100.0 + 7900.0 * i as f64 / 2999.0).collect();
    let fossa = (0..el.n_branches).find(|&j| el.b[j] == air.fossa.first_node()).unwrap();
    for (name, cut) in [("with fossae", vec![air.glottis_port]), ("no fossae", vec![air.glottis_port, fossa])] {
        let h = transfer_function(&el, air.pharynx.first_node(), &freqs, &cut);
        let peaks: Vec<String> = find_peaks(&freqs, &h.mag, 150.0).iter().take(5).map(|p| format!("{:.0}/{:.0}", p.freq, p.bw)).collect();
        let inv: Vec<f64> = h.mag.iter().map(|m| 1.0 / (m + 1e-30)).collect();
        let mut dips: Vec<(f64, f64)> = find_peaks(&freqs, &inv, 2000.0).iter().filter(|p| p.freq < 6000.0).map(|p| (p.freq, p.db)).collect();
        dips.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap());
        let dips: Vec<String> = dips.iter().take(3).map(|d| format!("{:.0}", d.0)).collect();
        println!("{body_name} /{ipa}/ {name:12} F(bw): {}   dips: {}", peaks.join("  "), dips.join(" "));
    }
}
