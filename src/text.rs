//! Utilidades de texto.

/// Minúsculas y sin tildes, para comparar.
pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::fold;

    #[test]
    fn fold_ignores_case_and_accents() {
        assert_eq!(fold("Señal Áreas"), "senal areas");
        assert!(fold("Configuración").contains(&fold("configuracion")));
    }
}
