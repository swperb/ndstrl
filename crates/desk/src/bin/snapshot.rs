//! `desk-snapshot <repo> <out.html> [--tab floor|receipts] [--theme light|dark]`
//! Static render of the front office for UI review and screenshot tests.

use ndstrl_desk::{model, snapshot_html, ui::Tab};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let pos: Vec<&String> = args.iter().enumerate().filter(|(i, a)| !a.starts_with("--") && (*i == 0 || !args[i - 1].starts_with("--"))).map(|(_, a)| a).collect();
    if pos.len() < 2 {
        eprintln!("usage: desk-snapshot <repo> <out.html> [--tab floor|receipts] [--theme light|dark]");
        std::process::exit(2);
    }
    let view = model::load(std::path::Path::new(pos[0]));
    let tab = match flag("--tab").as_deref() {
        Some("receipts") => Tab::Receipts,
        _ => Tab::Floor,
    };
    let html = snapshot_html(view, tab, flag("--theme").as_deref());
    std::fs::write(pos[1], html).expect("write snapshot");
    println!("{}", pos[1]);
}
