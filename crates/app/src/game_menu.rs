//! Menu do jogo em execução, aberto pelo Guide (como o overlay do Steam): voltar ao jogo,
//! minimizar ou fechar.

use bigpicture_core::Direction;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameAction {
    Resume,
    Minimize,
    Close,
}

impl GameAction {
    pub const ALL: [GameAction; 3] = [GameAction::Resume, GameAction::Minimize, GameAction::Close];

    pub fn label(self) -> &'static str {
        match self {
            GameAction::Resume => "Voltar ao jogo",
            GameAction::Minimize => "Minimizar jogo",
            GameAction::Close => "Fechar jogo",
        }
    }
}

/// Jogo aberto pelo launcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningGame {
    pub pid: u32,
    pub title: String,
}

/// Estado do menu aberto: qual opção está selecionada.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameMenu {
    selected: usize,
}

impl GameMenu {
    pub fn selected(self) -> GameAction {
        GameAction::ALL[self.selected]
    }

    /// Esquerda/cima vão para a opção anterior, direita/baixo para a próxima, sem dar a volta.
    pub fn step(&mut self, dir: Direction) {
        let last = GameAction::ALL.len() - 1;
        self.selected = match dir {
            Direction::Left | Direction::Up => self.selected.saturating_sub(1),
            Direction::Right | Direction::Down => (self.selected + 1).min(last),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_on_resume_and_moves_without_wrapping() {
        let mut menu = GameMenu::default();
        assert_eq!(menu.selected(), GameAction::Resume);
        menu.step(Direction::Left);
        assert_eq!(menu.selected(), GameAction::Resume);
        menu.step(Direction::Right);
        assert_eq!(menu.selected(), GameAction::Minimize);
        menu.step(Direction::Down);
        menu.step(Direction::Down);
        assert_eq!(menu.selected(), GameAction::Close);
        menu.step(Direction::Up);
        assert_eq!(menu.selected(), GameAction::Minimize);
    }
}
