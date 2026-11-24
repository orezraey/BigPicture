//! KeyValues da Valve: o formato texto (VDF/ACF, usado em `libraryfolders.vdf` e
//! `appmanifest_*.acf`) e o binário (cache de estatísticas em `appcache/stats/*.bin`).
//! Os dois viram a mesma árvore [`Kv`].

use std::collections::BTreeMap;

/// Valor de uma chave: texto, número ou um bloco de filhos (em ordem de chave).
#[derive(Clone, Debug, PartialEq)]
pub enum Kv {
    Str(String),
    Int(i64),
    Float(f32),
    Block(BTreeMap<String, Kv>),
}

impl Kv {
    /// Filho `key`, sem diferenciar maiúsculas (a Valve mistura `Name` e `name`).
    pub fn get(&self, key: &str) -> Option<&Kv> {
        let Kv::Block(map) = self else { return None };
        map.get(key)
            .or_else(|| map.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v))
    }

    /// Caminho de chaves, como `get("a")?.get("b")`.
    pub fn path(&self, keys: &[&str]) -> Option<&Kv> {
        keys.iter().try_fold(self, |node, key| node.get(key))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Kv::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Número, aceitando texto numérico (no VDF texto tudo é string).
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Kv::Int(n) => Some(*n),
            Kv::Str(s) => s.trim().parse().ok(),
            Kv::Float(f) => Some(*f as i64),
            Kv::Block(_) => None,
        }
    }

    pub fn children(&self) -> impl Iterator<Item = (&String, &Kv)> {
        let map = match self {
            Kv::Block(map) => Some(map),
            _ => None,
        };
        map.into_iter().flat_map(|m| m.iter())
    }
}

/// Lê VDF texto: `"chave" "valor"` e `"chave" { ... }`, com `//` comentários e escapes `\\`,
/// `\"`, `\n`, `\t`. Retorna o bloco raiz (que contém, por exemplo, `"AppState"`).
pub fn parse_text(input: &str) -> Result<Kv, String> {
    let mut tokens = tokenize(input)?.into_iter().peekable();
    let root = parse_block(&mut tokens, false)?;
    Ok(root)
}

#[derive(Debug, PartialEq)]
enum Token {
    Str(String),
    Open,
    Close,
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {}
            '{' => out.push(Token::Open),
            '}' => out.push(Token::Close),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut s = String::new();
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(other) => s.push(other),
                            None => return Err("escape no fim do texto".into()),
                        },
                        Some(other) => s.push(other),
                        None => return Err("aspas sem fechar".into()),
                    }
                }
                out.push(Token::Str(s));
            }
            // Chave sem aspas (aparece em alguns arquivos antigos): vai até espaço ou chave.
            other => {
                let mut s = String::from(other);
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                        break;
                    }
                    s.push(c);
                    chars.next();
                }
                out.push(Token::Str(s));
            }
        }
    }
    Ok(out)
}

fn parse_block(
    tokens: &mut std::iter::Peekable<std::vec::IntoIter<Token>>,
    nested: bool,
) -> Result<Kv, String> {
    let mut map = BTreeMap::new();
    loop {
        match tokens.next() {
            None if nested => return Err("bloco sem fechar".into()),
            None => return Ok(Kv::Block(map)),
            Some(Token::Close) if nested => return Ok(Kv::Block(map)),
            Some(Token::Close) => return Err("} sobrando".into()),
            Some(Token::Open) => return Err("{ sem chave".into()),
            Some(Token::Str(key)) => {
                let value = match tokens.next() {
                    Some(Token::Str(v)) => Kv::Str(v),
                    Some(Token::Open) => parse_block(tokens, true)?,
                    Some(Token::Close) | None => return Err(format!("chave {key:?} sem valor")),
                };
                map.insert(key, value);
            }
        }
    }
}

/// Lê KeyValues binário: cada item é `tipo(u8) nome(C string) valor`; tipo 0 abre um bloco, 1
/// é string, 2 é i32, 3 é f32, 7 é u64 e 8 fecha o bloco atual.
pub fn parse_binary(data: &[u8]) -> Result<Kv, String> {
    let mut pos = 0;
    let root = parse_binary_block(data, &mut pos)?;
    Ok(root)
}

fn parse_binary_block(data: &[u8], pos: &mut usize) -> Result<Kv, String> {
    let mut map = BTreeMap::new();
    loop {
        let Some(&kind) = data.get(*pos) else {
            // Fim dos dados também fecha o bloco raiz.
            return Ok(Kv::Block(map));
        };
        *pos += 1;
        if kind == 8 || kind == 11 {
            return Ok(Kv::Block(map));
        }
        let key = read_cstr(data, pos)?;
        let value = match kind {
            0 => parse_binary_block(data, pos)?,
            1 => Kv::Str(read_cstr(data, pos)?),
            2 | 4 | 6 => Kv::Int(i64::from(i32::from_le_bytes(read_n::<4>(data, pos)?))),
            3 => Kv::Float(f32::from_le_bytes(read_n::<4>(data, pos)?)),
            7 => Kv::Int(u64::from_le_bytes(read_n::<8>(data, pos)?) as i64),
            10 => Kv::Int(i64::from_le_bytes(read_n::<8>(data, pos)?)),
            other => return Err(format!("tipo {other} desconhecido em {}", *pos - 1)),
        };
        map.insert(key, value);
    }
}

fn read_cstr(data: &[u8], pos: &mut usize) -> Result<String, String> {
    let rest = data.get(*pos..).ok_or("texto além do fim")?;
    let len = rest.iter().position(|&b| b == 0).ok_or("texto sem terminador")?;
    let s = String::from_utf8_lossy(&rest[..len]).into_owned();
    *pos += len + 1;
    Ok(s)
}

fn read_n<const N: usize>(data: &[u8], pos: &mut usize) -> Result<[u8; N], String> {
    let bytes = data.get(*pos..*pos + N).ok_or("número além do fim")?;
    *pos += N;
    Ok(bytes.try_into().expect("tamanho conferido"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_vdf_with_nesting_comments_and_escapes() {
        let kv = parse_text(
            r#"
            // comentário
            "libraryfolders"
            {
                "0"
                {
                    "path"  "C:\\Program Files (x86)\\Steam"
                    "apps" { "228980" "245987471" "1454400" "226758747" }
                }
                "1" { "path" "D:\\SteamLibrary" }
            }"#,
        )
        .unwrap();
        let lf = kv.get("libraryfolders").unwrap();
        assert_eq!(lf.path(&["0", "path"]).and_then(Kv::as_str), Some(r"C:\Program Files (x86)\Steam"));
        assert_eq!(lf.path(&["1", "PATH"]).and_then(Kv::as_str), Some(r"D:\SteamLibrary"));
        assert_eq!(lf.path(&["0", "apps", "1454400"]).and_then(Kv::as_i64), Some(226758747));
        assert_eq!(lf.children().count(), 2);
    }

    #[test]
    fn rejects_broken_text_vdf() {
        assert!(parse_text(r#""a" { "b" "c" "#).is_err());
        assert!(parse_text(r#""a" "#).is_err());
        assert!(parse_text(r#""a" "b" }"#).is_err());
    }

    fn bin_str(out: &mut Vec<u8>, key: &str, value: &str) {
        out.push(1);
        out.extend_from_slice(key.as_bytes());
        out.push(0);
        out.extend_from_slice(value.as_bytes());
        out.push(0);
    }

    #[test]
    fn parses_binary_keyvalues() {
        let mut data = vec![0];
        data.extend_from_slice(b"1454400\0");
        bin_str(&mut data, "gamename", "Cookie Clicker");
        data.push(2);
        data.extend_from_slice(b"version\0");
        data.extend_from_slice(&7i32.to_le_bytes());
        data.push(0);
        data.extend_from_slice(b"stats\0");
        data.push(8);
        data.push(8);
        data.push(8);
        let kv = parse_binary(&data).unwrap();
        let game = kv.get("1454400").unwrap();
        assert_eq!(game.get("gamename").and_then(Kv::as_str), Some("Cookie Clicker"));
        assert_eq!(game.get("version").and_then(Kv::as_i64), Some(7));
        assert_eq!(game.get("stats"), Some(&Kv::Block(BTreeMap::new())));
        assert!(parse_binary(&[1, b'a']).is_err());
    }
}
