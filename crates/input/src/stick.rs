//! Analógico esquerdo como direcional, com histerese para não "tremer" perto do limite.

use crate::NavCommand;

/// Inclinação a partir da qual o analógico vira um direcional pressionado.
pub const PRESS_THRESHOLD: f32 = 0.6;
/// Inclinação abaixo da qual o direcional do analógico é solto.
pub const RELEASE_THRESHOLD: f32 = 0.35;

/// Mudança de estado do direcional virtual do analógico.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickEdge {
    Pressed(NavCommand),
    Released(NavCommand),
}

/// Estado do analógico. Os eixos seguem a convenção do gilrs: Y positivo é para cima.
#[derive(Debug, Default)]
pub struct StickState {
    x: f32,
    y: f32,
    active: Option<NavCommand>,
}

impl StickState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_x(&mut self, x: f32) -> Vec<StickEdge> {
        self.x = x;
        self.update()
    }

    pub fn set_y(&mut self, y: f32) -> Vec<StickEdge> {
        self.y = y;
        self.update()
    }

    fn update(&mut self) -> Vec<StickEdge> {
        let mut edges = Vec::new();
        // O direcional atual continua até a inclinação no seu eixo cair abaixo do limite de
        // soltura, mesmo que o outro eixo passe um pouco dele.
        if let Some(cmd) = self.active {
            if self.magnitude_toward(cmd) >= RELEASE_THRESHOLD && self.dominant() == Some(cmd) {
                return edges;
            }
            edges.push(StickEdge::Released(cmd));
            self.active = None;
        }
        if let Some(cmd) = self.dominant()
            && self.magnitude_toward(cmd) >= PRESS_THRESHOLD
        {
            self.active = Some(cmd);
            edges.push(StickEdge::Pressed(cmd));
        }
        edges
    }

    fn dominant(&self) -> Option<NavCommand> {
        if self.x == 0.0 && self.y == 0.0 {
            return None;
        }
        Some(if self.x.abs() >= self.y.abs() {
            if self.x > 0.0 { NavCommand::Right } else { NavCommand::Left }
        } else if self.y > 0.0 {
            NavCommand::Up
        } else {
            NavCommand::Down
        })
    }

    fn magnitude_toward(&self, cmd: NavCommand) -> f32 {
        match cmd {
            NavCommand::Right => self.x,
            NavCommand::Left => -self.x,
            NavCommand::Up => self.y,
            NavCommand::Down => -self.y,
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presses_past_threshold_and_releases_with_hysteresis() {
        let mut s = StickState::new();
        assert!(s.set_x(0.5).is_empty());
        assert_eq!(s.set_x(0.7), vec![StickEdge::Pressed(NavCommand::Right)]);
        assert!(s.set_x(0.45).is_empty(), "entre os limites continua pressionado");
        assert_eq!(s.set_x(0.2), vec![StickEdge::Released(NavCommand::Right)]);
    }

    #[test]
    fn y_up_is_positive_and_diagonal_switches_cleanly() {
        let mut s = StickState::new();
        assert_eq!(s.set_y(-0.9), vec![StickEdge::Pressed(NavCommand::Down)]);
        assert_eq!(
            s.set_x(-1.0),
            vec![StickEdge::Released(NavCommand::Down), StickEdge::Pressed(NavCommand::Left)]
        );
        s.set_x(0.0);
        let mut s = StickState::new();
        assert_eq!(s.set_y(0.8), vec![StickEdge::Pressed(NavCommand::Up)]);
    }
}
