//! Mostra a árvore de um arquivo KeyValues (texto ou binário): `cargo run -p bigpicture-providers --example kvdump -- ARQUIVO [profundidade]`.
use bigpicture_providers::keyvalues::{Kv, parse_binary, parse_text};

fn show(kv: &Kv, depth: usize, max: usize, indent: usize) {
    for (k, v) in kv.children() {
        match v {
            Kv::Block(_) if depth + 1 < max => {
                println!("{:indent$}{k}", "");
                show(v, depth + 1, max, indent + 2);
            }
            Kv::Block(m) => println!("{:indent$}{k} {{{} filhos}}", "", m.len()),
            other => println!("{:indent$}{k} = {other:?}", ""),
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = &args[0];
    let max = args.get(1).and_then(|d| d.parse().ok()).unwrap_or(4);
    let bytes = std::fs::read(path).expect("ler arquivo");
    let kv = if path.ends_with(".bin") { parse_binary(&bytes) } else { parse_text(&String::from_utf8_lossy(&bytes)) }.expect("parse");
    show(&kv, 0, max, 0);
}
