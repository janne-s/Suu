//! Where does a resonance come from? The airway's transfer function (glottal flow to radiated
//! pressure) for one calibrated vowel, with parts of the airway switched off one at a time.
//!
//! `cargo run --release -q -p chimera --example tract_tf -- [male|female] [ipa] [fs]`
//! (fs is the airway's own rate: 96000 for a 48 kHz voice at 2× oversampling)

use chimera::airway::frequency::{find_peaks, transfer_function};
use chimera::airway::network::LinearElements;
use chimera::anatomy::human::{ArticKey, BODY_FEMALE, BODY_MALE, NEUTRAL};
use chimera::phonetics::planner::vowel_table;
use chimera::voice::human_airway::HumanAirway;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let body_name = args.first().map(String::as_str).unwrap_or("female");
    let ipa = args.get(1).map(String::as_str).unwrap_or("ɔ");
    let fs: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(96000.0);
    let body = if body_name == "male" { BODY_MALE } else { BODY_FEMALE };

    let mut air = HumanAirway::new(fs, body);
    let mut a = NEUTRAL;
    let target = &vowel_table()[body_name][ipa];
    for k in ArticKey::ALL {
        if let Some(v) = target.artic.get(k) {
            *a.get_mut(k) = v;
        }
    }
    air.apply(&a, 0.0);
    let el = air.net.linear_elements();
    let n = 2000;
    let freqs: Vec<f64> = (0..n).map(|i| 100.0 + (8000.0 - 100.0) * i as f64 / (n - 1) as f64).collect();
    let inject = air.pharynx.first_node();
    let fossa_branch = (0..el.n_branches).find(|&j| el.b[j] == air.fossa.first_node()).expect("fossa branch");

    let mut no_wall_rad = el.clone();
    no_wall_rad.wall_rad.iter_mut().for_each(|x| *x = 0.0);
    let mut rigid = el.clone();
    rigid.wall_area.iter_mut().for_each(|x| *x = 0.0);

    println!("{body_name} /{ipa}/, airway fs {fs} Hz, {} pharynx + {} oral nodes, velum port open: {}", air.n_ph, air.n_or, el.open[air.velum_port]);
    let cases: [(&str, &LinearElements, Vec<usize>); 5] = [
        ("full model", &el, vec![air.glottis_port]),
        ("piriform fossae removed", &el, vec![air.glottis_port, fossa_branch]),
        ("no wall (skin) radiation", &no_wall_rad, vec![air.glottis_port]),
        ("rigid walls", &rigid, vec![air.glottis_port]),
        ("fossae removed + no wall radiation", &no_wall_rad, vec![air.glottis_port, fossa_branch]),
    ];
    let full = transfer_function(&el, inject, &freqs, &[air.glottis_port]);
    let band = |db: &[f64], lo: f64, hi: f64| {
        let v: Vec<f64> = freqs.iter().zip(db).filter(|(f, _)| **f >= lo && **f < hi).map(|(_, d)| *d).collect();
        v.iter().sum::<f64>() / v.len() as f64
    };
    let ref_db = band(&full.pressure_db, 300.0, 1000.0);
    // wall losses: the engine evaluates the boundary-layer viscous loss at one frequency
    // (viscous_omega, 700 Hz). The real loss grows as sqrt(f), and thermal conduction adds about
    // (γ−1)/√Pr ≈ 0.47 of it to the attenuation. Rebuild the elements per frequency to see the
    // physical reference, and a fixed reference frequency the time-domain model could use.
    let loss_case = |air: &mut HumanAirway, scale: f64, omega: Option<f64>| {
        let mut mag = Vec::with_capacity(n);
        let mut pdb = Vec::with_capacity(n);
        for &f in &freqs {
            air.net.viscous_scale = scale;
            air.net.viscous_omega = omega.unwrap_or(2.0 * std::f64::consts::PI * f);
            let e = air.net.linear_elements();
            let h = transfer_function(&e, inject, &[f], &[air.glottis_port]);
            mag.push(h.mag[0]);
            pdb.push(h.pressure_db[0]);
        }
        (mag, pdb)
    };
    let pi2 = 2.0 * std::f64::consts::PI;
    // extra cases from the command line: scale@refHz,scale@refHz,…
    let extra: Vec<(String, f64, Option<f64>)> = args.get(3).map(|s| s.split(',').filter_map(|c| {
        let (sc, hz) = c.split_once('@')?;
        Some((format!("losses: ×{sc} at {hz} Hz"), sc.parse().ok()?, Some(pi2 * hz.parse::<f64>().ok()?)))
    }).collect()).unwrap_or_default();
    let loss_cases: [(&str, f64, Option<f64>); 5] = [
        ("losses: engine now (viscous at 700 Hz)", 1.0, Some(pi2 * 700.0)),
        ("losses: viscous ∝ √f (physical)", 1.0, None),
        ("losses: viscous ∝ √f + thermal (physical)", 1.47, None),
        ("losses: fixed at 1.5 kHz, + thermal", 1.47, Some(pi2 * 1500.0)),
        ("losses: fixed at 2.5 kHz, + thermal", 1.47, Some(pi2 * 2500.0)),
    ];
    let all: Vec<(String, f64, Option<f64>)> = loss_cases.iter().map(|(n, s, o)| (n.to_string(), *s, *o)).chain(extra).collect();
    for (name, scale, omega) in all {
        let (mag, pdb) = loss_case(&mut air, scale, omega);
        println!("\n== {name}");
        println!("   pressure re full 0.3–1 kHz: 2–3k {:+.1}  3–4k {:+.1}  4–5k {:+.1}  5–6k {:+.1}  6–8k {:+.1} dB",
            band(&pdb, 2000.0, 3000.0) - ref_db, band(&pdb, 3000.0, 4000.0) - ref_db,
            band(&pdb, 4000.0, 5000.0) - ref_db, band(&pdb, 5000.0, 6000.0) - ref_db, band(&pdb, 6000.0, 8000.0) - ref_db);
        for p in find_peaks(&freqs, &mag, 150.0) {
            println!("   peak {:6.0} Hz  bw {:5.0} Hz", p.freq, p.bw);
        }
    }
    air.net.viscous_scale = 1.0;
    air.net.viscous_omega = pi2 * 700.0;
    for (name, e, cut) in cases {
        let h = transfer_function(e, inject, &freqs, &cut);
        let peaks = find_peaks(&freqs, &h.mag, 150.0);
        println!("\n== {name}");
        println!("   pressure re full 0.3–1 kHz: 2–3k {:+.1}  3–4k {:+.1}  4–5k {:+.1}  5–6k {:+.1}  6–8k {:+.1} dB",
            band(&h.pressure_db, 2000.0, 3000.0) - ref_db, band(&h.pressure_db, 3000.0, 4000.0) - ref_db,
            band(&h.pressure_db, 4000.0, 5000.0) - ref_db, band(&h.pressure_db, 5000.0, 6000.0) - ref_db,
            band(&h.pressure_db, 6000.0, 8000.0) - ref_db);
        for p in peaks {
            println!("   peak {:6.0} Hz  bw {:5.0} Hz", p.freq, p.bw);
        }
    }
}
