//! Origem de um item da biblioteca (loja) e conquistas.

/// De onde veio o item do catálogo.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum GameSource {
    /// Arquivo local, app conhecido ou item de demonstração.
    #[default]
    Local,
    Steam {
        app_id: u32,
    },
    Epic {
        /// `AppName` do manifesto (o identificador interno da Epic).
        app_name: String,
    },
}

impl GameSource {
    pub fn store_name(&self) -> &'static str {
        match self {
            GameSource::Local => "Local",
            GameSource::Steam { .. } => "Steam",
            GameSource::Epic { .. } => "Epic Games",
        }
    }
}

/// Uma conquista de um jogo, com o progresso do usuário.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Achievement {
    /// Identificador da loja (estável entre idiomas).
    pub api_name: String,
    pub name: String,
    pub description: String,
    /// Conquista secreta: a descrição só deve aparecer depois de desbloqueada.
    pub hidden: bool,
    /// Momento do desbloqueio (segundos Unix), `None` se ainda bloqueada.
    pub unlocked_at: Option<u64>,
    /// Endereço do ícone colorido e do cinza (bloqueada), quando a loja informa.
    pub icon_url: Option<String>,
    pub icon_locked_url: Option<String>,
}

impl Achievement {
    pub fn unlocked(&self) -> bool {
        self.unlocked_at.is_some()
    }
}

/// Resumo de progresso: quantas desbloqueadas de quantas.
pub fn progress(achievements: &[Achievement]) -> (usize, usize) {
    (achievements.iter().filter(|a| a.unlocked()).count(), achievements.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ach(name: &str, at: Option<u64>) -> Achievement {
        Achievement {
            api_name: name.into(),
            name: name.into(),
            description: String::new(),
            hidden: false,
            unlocked_at: at,
            icon_url: None,
            icon_locked_url: None,
        }
    }

    #[test]
    fn counts_unlocked_achievements() {
        let list = [ach("a", Some(1)), ach("b", None), ach("c", Some(5))];
        assert_eq!(progress(&list), (2, 3));
        assert_eq!(progress(&[]), (0, 0));
        assert_eq!(GameSource::Steam { app_id: 1 }.store_name(), "Steam");
        assert_eq!(GameSource::default(), GameSource::Local);
    }
}
