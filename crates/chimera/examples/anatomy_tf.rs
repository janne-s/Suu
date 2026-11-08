//! F1–F4 of the calibrated vowels on a body with chosen proportions, for calibrating the anatomy
//! against sung vowels (Doc/reviews/2026-11-08-anatomy). Frequency-domain oracle, no audio.
//!
//! `cargo run --release -q -p chimera --example anatomy_tf -- <male|female table> <scale> <pharynx>
//!  <tongue_radius> <lip_width> [vowels, default ɑ,e,i,o,u] [epi_area]`
//! Prints one line per vowel: F1 F2 F3 F4 (Hz).

use chimera::airway::frequency::{find_peaks, transfer_function};
use chimera::anatomy::human::{ArticKey, HumanBody, NEUTRAL};
use chimera::phonetics::planner::vowel_table;
use chimera::voice::human_airway::HumanAirway;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let table = a[0].as_str();
    let num = |i: usize| a[i].parse::<f64>().expect("number");
    let body = HumanBody { name: "custom", scale: num(1), pharynx: num(2), tongue_radius: num(3), lip_width: num(4), epi_area: a.get(6).and_then(|x| x.parse().ok()).unwrap_or(1.0) };
    let vowels = a.get(5).map(String::as_str).unwrap_or("ɑ,e,i,o,u");
    let mut air = HumanAirway::new(96000.0, body);
    let freqs: Vec<f64> = (0..2400).map(|i| 150.0 + 4850.0 * i as f64 / 2399.0).collect();
    for v in vowels.split(',') {
        let mut art = NEUTRAL;
        for k in ArticKey::ALL {
            if let Some(x) = vowel_table()[table][v].artic.get(k) {
                *art.get_mut(k) = x;
            }
        }
        air.apply(&art, 0.0);
        let el = air.net.linear_elements();
        let h = transfer_function(&el, air.pharynx.first_node(), &freqs, &[air.glottis_port]);
        let f: Vec<String> = find_peaks(&freqs, &h.mag, 150.0).iter().take(4).map(|p| format!("{:.0}", p.freq)).collect();
        println!("{v} {}", f.join(" "));
    }
}
