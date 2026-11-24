//! Tela de taxa de atualização (botão Menu): lista as taxas do monitor do launcher na
//! resolução atual, aplica a escolhida e pede confirmação, voltando sozinha se ninguém
//! confirmar a tempo (como o Windows faz ao trocar de modo).

use std::time::{Duration, Instant};

use bigpicture_core::Direction;
use bigpicture_system::{DisplayInfo, DisplayMode};

/// Tempo para confirmar a nova taxa antes de voltar à anterior.
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(10);

/// Uma opção da lista: uma taxa na resolução atual, ou voltar ao modo salvo do sistema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateOption {
    Mode(DisplayMode),
    SystemDefault,
}

impl RateOption {
    pub fn label(self) -> String {
        match self {
            RateOption::Mode(mode) => format!("{} Hz", mode.refresh_hz()),
            RateOption::SystemDefault => "Padrão do sistema".to_string(),
        }
    }
}

/// Troca aplicada esperando confirmação.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingChange {
    pub previous: DisplayMode,
    pub applied: DisplayMode,
    pub deadline: Instant,
    /// Na confirmação: `true` com "Manter" selecionado, `false` com "Voltar".
    pub keep_selected: bool,
}

impl PendingChange {
    pub fn seconds_left(&self, now: Instant) -> u64 {
        self.deadline.saturating_duration_since(now).as_secs_f32().ceil() as u64
    }

    pub fn expired(&self, now: Instant) -> bool {
        now >= self.deadline
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayMenu {
    pub display: DisplayInfo,
    pub options: Vec<RateOption>,
    pub selected: usize,
    pub pending: Option<PendingChange>,
}

impl DisplayMenu {
    /// Taxas oferecidas na resolução atual (da maior para a menor) e, por último, o padrão do
    /// sistema. A seleção começa na taxa atual.
    pub fn new(display: DisplayInfo) -> Self {
        let current = display.current;
        let mut rates: Vec<DisplayMode> = display
            .modes
            .iter()
            .copied()
            .filter(|m| m.width() == current.width() && m.height() == current.height())
            .collect();
        rates.sort_by_key(|m| std::cmp::Reverse(m.refresh_hz()));
        rates.dedup_by_key(|m| m.refresh_hz());
        let mut options: Vec<RateOption> = rates.into_iter().map(RateOption::Mode).collect();
        options.push(RateOption::SystemDefault);
        let selected = options
            .iter()
            .position(|o| *o == RateOption::Mode(current))
            .unwrap_or(0);
        Self { display, options, selected, pending: None }
    }

    pub fn selected_option(&self) -> RateOption {
        self.options[self.selected]
    }

    /// Move a seleção (ou alterna Manter/Voltar durante a confirmação).
    pub fn step(&mut self, dir: Direction) {
        if let Some(pending) = &mut self.pending {
            pending.keep_selected = matches!(dir, Direction::Left | Direction::Up);
            return;
        }
        let last = self.options.len() - 1;
        self.selected = match dir {
            Direction::Left | Direction::Up => self.selected.saturating_sub(1),
            Direction::Right | Direction::Down => (self.selected + 1).min(last),
        };
    }

    /// Registra que `applied` entrou no lugar do modo atual e começa a contagem.
    pub fn start_confirmation(&mut self, applied: DisplayMode, now: Instant) {
        self.pending = Some(PendingChange {
            previous: self.display.current,
            applied,
            deadline: now + CONFIRM_TIMEOUT,
            keep_selected: true,
        });
        self.display.current = applied;
    }

    /// Fecha a confirmação. Com `keep`, a nova taxa vira a atual; sem, devolve o modo para o
    /// qual voltar.
    pub fn finish_confirmation(&mut self, keep: bool) -> Option<DisplayMode> {
        let pending = self.pending.take()?;
        if keep {
            None
        } else {
            self.display.current = pending.previous;
            Some(pending.previous)
        }
    }
}

#[cfg(test)]
mod tests {
    use bigpicture_system::DisplayId;

    use super::*;

    fn display() -> DisplayInfo {
        DisplayInfo {
            id: DisplayId(r"\\.\DISPLAY1".into()),
            name: "GPU".into(),
            primary: true,
            position: (0, 0),
            current: DisplayMode::new(3440, 1440, 240),
            modes: vec![
                DisplayMode::new(3840, 2160, 60),
                DisplayMode::new(3440, 1440, 240),
                DisplayMode::new(3440, 1440, 144),
                DisplayMode::new(3440, 1440, 60),
                DisplayMode::new(1920, 1080, 120),
            ],
        }
    }

    #[test]
    fn lists_rates_of_current_resolution_plus_system_default() {
        let menu = DisplayMenu::new(display());
        let labels: Vec<String> = menu.options.iter().map(|o| o.label()).collect();
        assert_eq!(labels, ["240 Hz", "144 Hz", "60 Hz", "Padrão do sistema"]);
        assert_eq!(menu.selected_option(), RateOption::Mode(DisplayMode::new(3440, 1440, 240)));
    }

    #[test]
    fn confirmation_counts_down_and_reverts_unless_kept() {
        let t0 = Instant::now();
        let mut menu = DisplayMenu::new(display());
        menu.step(Direction::Right);
        let RateOption::Mode(mode) = menu.selected_option() else { panic!() };
        menu.start_confirmation(mode, t0);
        let pending = menu.pending.unwrap();
        assert_eq!(pending.seconds_left(t0), 10);
        assert!(!pending.expired(t0 + Duration::from_secs(9)));
        assert!(pending.expired(t0 + CONFIRM_TIMEOUT));
        assert_eq!(menu.display.current.refresh_hz(), 144);

        menu.step(Direction::Right);
        assert!(!menu.pending.unwrap().keep_selected, "direita seleciona Voltar");
        assert_eq!(menu.finish_confirmation(false), Some(DisplayMode::new(3440, 1440, 240)));
        assert_eq!(menu.display.current.refresh_hz(), 240);
        assert!(menu.pending.is_none());

        menu.start_confirmation(mode, t0);
        assert_eq!(menu.finish_confirmation(true), None);
        assert_eq!(menu.display.current.refresh_hz(), 144);
    }
}
