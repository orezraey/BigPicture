//! Menu de configurações (botão Menu) e a pergunta de permissão da biblioteca da Steam.

use bigpicture_core::Direction;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsItem {
    RefreshRate,
    SteamFullLibrary,
}

/// Lista de itens com um selecionado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsMenu {
    pub items: Vec<SettingsItem>,
    pub selected: usize,
}

impl SettingsMenu {
    /// O item da Steam só aparece quando a Steam está instalada.
    pub fn new(steam_installed: bool) -> Self {
        let mut items = vec![SettingsItem::RefreshRate];
        if steam_installed {
            items.push(SettingsItem::SteamFullLibrary);
        }
        Self { items, selected: 0 }
    }

    pub fn selected_item(&self) -> SettingsItem {
        self.items[self.selected]
    }

    pub fn step(&mut self, dir: Direction) {
        let last = self.items.len() - 1;
        self.selected = match dir {
            Direction::Up | Direction::Left => self.selected.saturating_sub(1),
            Direction::Down | Direction::Right => (self.selected + 1).min(last),
        };
    }
}

/// Pergunta "Ler sua biblioteca completa da Steam?": `true` com "Permitir" selecionado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SteamPrompt {
    pub allow_selected: bool,
}

impl SteamPrompt {
    pub fn new() -> Self {
        Self { allow_selected: true }
    }

    pub fn step(&mut self, dir: Direction) {
        self.allow_selected = matches!(dir, Direction::Left | Direction::Up);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_item_only_when_installed_and_selection_is_clamped() {
        assert_eq!(SettingsMenu::new(false).items, [SettingsItem::RefreshRate]);
        let mut menu = SettingsMenu::new(true);
        menu.step(Direction::Down);
        menu.step(Direction::Down);
        assert_eq!(menu.selected_item(), SettingsItem::SteamFullLibrary);
        menu.step(Direction::Up);
        assert_eq!(menu.selected_item(), SettingsItem::RefreshRate);
    }

    #[test]
    fn prompt_starts_on_allow_and_right_moves_to_not_now() {
        let mut prompt = SteamPrompt::new();
        assert!(prompt.allow_selected);
        prompt.step(Direction::Right);
        assert!(!prompt.allow_selected);
        prompt.step(Direction::Left);
        assert!(prompt.allow_selected);
    }
}
