//! Modelli di email salvati come file di testo con segnaposto {nome}.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const DEFAULTS: &[(&str, &str)] = &[
    ("risposta_generica.txt", "Gentile {nome},\n\nGrazie per la tua email del {data}.\n\nTi risponderò a breve.\n\nCordiali saluti,\n{nome_mittente}"),
    ("pec_ricevuta.txt", "Confermo ricezione PEC del {data}.\n\nPratica: {oggetto}\n\nDistinti saluti"),
    ("fuori_ufficio.txt", "Sono fuori ufficio dal {da} al {a}.\n\nRisponderò al mio rientro.\n\nPer urgenze contattare {contatto}."),
    ("fattura.txt", "Gentile {cliente},\n\nIn allegato trovi la fattura n. {numero_fattura} del {data_fattura}.\n\nCordiali saluti,\n{mittente}"),
];

pub fn ensure_defaults(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cartella modelli: {e}"))?;
    for (name, text) in DEFAULTS {
        let p = dir.join(name);
        if !p.exists() {
            std::fs::write(&p, text).map_err(|e| format!("modello {name}: {e}"))?;
        }
    }
    Ok(())
}

pub fn list(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p: &PathBuf| p.extension().is_some_and(|x| x == "txt"))
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    v.sort();
    v
}

pub fn load(dir: &Path, name: &str) -> Option<String> {
    // Solo il nome del file: niente percorsi.
    if name.contains(['/', '\\']) || name.contains("..") {
        return None;
    }
    std::fs::read_to_string(dir.join(name)).ok()
}

/// Nomi dei segnaposto `{nome}` nel testo, nell'ordine e senza ripetizioni.
pub fn placeholders(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(a) = rest.find('{') {
        let after = rest.get(a + 1..).unwrap_or("");
        let Some(b) = after.find('}') else { break };
        let name = after.get(..b).unwrap_or("");
        if !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !out.iter().any(|x| x == name)
        {
            out.push(name.to_string());
        }
        rest = after.get(b + 1..).unwrap_or("");
    }
    out
}

/// Sostituisce i segnaposto noti; gli altri restano com'erano.
pub fn fill(text: &str, values: &HashMap<String, String>) -> String {
    let mut out = text.to_string();
    for (k, v) in values {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_list_load_and_fill() {
        let d = std::env::temp_dir().join(format!("dove_t_{}", std::process::id()));
        ensure_defaults(&d).unwrap();
        std::fs::write(d.join("risposta_generica.txt"), "modificato {nome}").unwrap();
        ensure_defaults(&d).unwrap(); // non sovrascrive
        let l = list(&d);
        assert_eq!(l.len(), 4);
        let t = load(&d, "risposta_generica.txt").unwrap();
        assert_eq!(t, "modificato {nome}");
        assert!(load(&d, "../x").is_none() && load(&d, "manca.txt").is_none());
        let vals: HashMap<_, _> = [("nome".to_string(), "Anna".to_string())].into();
        assert_eq!(fill(&t, &vals), "modificato Anna");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn placeholder_parsing() {
        assert_eq!(
            placeholders("Ciao {a} e {b}, {a} {} { x } {c"),
            vec!["a", "b"]
        );
        assert!(placeholders("niente").is_empty());
        let v: HashMap<_, _> = [("a".to_string(), "1".to_string())].into();
        assert_eq!(fill("{a} {b}", &v), "1 {b}");
    }
}
