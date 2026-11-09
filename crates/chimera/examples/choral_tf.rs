//! Formants (peak / −3 dB bandwidth) of one vowel exactly as a `render notes` experiment file
//! sets it up: its "body", "vowelTable" (a `calibrate choral` table), "articOffset" and
//! "airway.viscousScale"/"viscousRefHz". Frequency-domain oracle of the airway (no audio), for
//! reading what a fitted posture did to F1–F5 (Doc/reviews/2026-11-08-choral-vowel).
//!
//! `cargo run --release -q -p chimera --example choral_tf -- <male|female> <render.json> [ipa, default the file's nucleus]`

use chimera::airway::frequency::{find_peaks, transfer_function};
use chimera::anatomy::human::{ArticKey, HumanBody, BODY_FEMALE, BODY_MALE, NEUTRAL};
use chimera::phonetics::planner::vowel_table;
use chimera::voice::human_airway::HumanAirway;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let body_name = a[0].as_str();
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&a[1]).expect("render file")).expect("json");
    let ipa = a.get(2).cloned().unwrap_or_else(|| j["nucleus"].as_str().expect("nucleus").to_string());
    let b0 = if body_name == "male" { BODY_MALE } else { BODY_FEMALE };
    let body = match j.get("body") {
        Some(o) => HumanBody {
            name: "custom",
            scale: o["scale"].as_f64().unwrap_or(b0.scale),
            pharynx: o["pharynx"].as_f64().unwrap_or(b0.pharynx),
            tongue_radius: o["tongueRadius"].as_f64().unwrap_or(b0.tongue_radius),
            lip_width: o["lipWidth"].as_f64().unwrap_or(b0.lip_width),
            epi_area: o["epiArea"].as_f64().unwrap_or(b0.epi_area),
        },
        None => b0,
    };
    let mut air = HumanAirway::new(96000.0, body);
    if let Some(x) = j["airway"]["viscousScale"].as_f64() {
        air.net.viscous_scale = x;
    }
    if let Some(hz) = j["airway"]["viscousRefHz"].as_f64() {
        air.net.viscous_omega = 2.0 * chimera::math::PI * hz;
    }
    if let Some(on) = j["airway"]["sqrtLosses"].as_bool() {
        air.net.sqrt_losses = on;
    }
    if let Some(k) = j["airway"]["hfLoss"].as_f64() {
        air.net.hf_loss = k;
    }
    if let Some(k) = j["airway"]["midLoss"].as_f64() {
        air.net.mid_loss = k;
    }
    if let Some(on) = j["airway"]["thermalLosses"].as_bool() {
        air.net.thermal_losses = on;
    }
    if let Some(k) = j["airway"]["wallRScale"].as_f64() {
        air.net.wall_r_scale = k;
    }
    let mut art = NEUTRAL;
    for k in ArticKey::ALL {
        if let Some(x) = vowel_table()[body_name][ipa.as_str()].artic.get(k) {
            *art.get_mut(k) = x;
        }
    }
    if let Some(path) = j["vowelTable"].as_str() {
        let t: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).expect("vowel table")).expect("json");
        if let Some(o) = t[ipa.as_str()]["artic"].as_object() {
            for (name, x) in o {
                if let (Some(k), Some(x)) = (ArticKey::from_name(name), x.as_f64()) {
                    *art.get_mut(k) = x;
                }
            }
        }
    }
    if let Some(o) = j["articOffset"].as_object() {
        for (name, d) in o {
            let k = ArticKey::from_name(name).unwrap_or_else(|| panic!("unknown articulator {name}"));
            *art.get_mut(k) += d.as_f64().expect("number");
        }
    }
    air.apply(&art, 0.0);
    let el = air.net.linear_elements();
    let freqs: Vec<f64> = (0..3000).map(|i| 100.0 + 5900.0 * i as f64 / 2999.0).collect();
    let h = transfer_function(&el, air.pharynx.first_node(), &freqs, &[air.glottis_port]);
    let pk = find_peaks(&freqs, &h.mag, 150.0);
    let f1_db = pk.first().map(|p| p.db).unwrap_or(0.0);
    let peaks: Vec<String> = pk.iter().take(5).map(|p| format!("{:.0}/{:.0}/{:+.0}", p.freq, p.bw, p.db - f1_db)).collect();
    // singer's-formant region: mean power 2.2–3.6 kHz re F1's peak (dB)
    let (mut s, mut n) = (0.0, 0.0);
    for (f, m) in freqs.iter().zip(&h.mag) {
        if (2200.0..3600.0).contains(f) {
            s += m * m;
            n += 1.0;
        }
    }
    let sf_db = 10.0 * (s / n).log10() - f1_db;
    println!("/{ipa}/ F/bw/dB re F1: {}   2.2–3.6 kHz {sf_db:+.1}", peaks.join("  "));
    if std::env::var("CHORAL_TF_AREAS").is_ok() {
        let rad: Vec<String> = el.rad_area.iter().map(|a| format!("{a:.2}")).collect();
        println!("   radiating areas (cm², lips then nostrils): {}", rad.join(" "));
    }
    // optional: the whole curve (Hz, dB) for comparing against a render
    if let Ok(path) = std::env::var("CHORAL_TF_CURVE") {
        let s: String = freqs.iter().zip(&h.mag).map(|(f, m)| format!("{f:.1} {:.2}\n", 20.0 * m.log10())).collect();
        std::fs::write(path, s).expect("write curve");
    }
}
