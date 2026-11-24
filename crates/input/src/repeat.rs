//! Repetição ao segurar um direcional: dispara na hora, espera [`INITIAL_DELAY`] e depois
//! repete a cada [`REPEAT_INTERVAL`] enquanto o mesmo direcional continuar pressionado.

use std::time::{Duration, Instant};

use crate::NavCommand;

/// Espera antes da primeira repetição (próximo do atraso de repetição de teclado do sistema).
pub const INITIAL_DELAY: Duration = Duration::from_millis(350);
/// Intervalo entre repetições depois do atraso inicial.
pub const REPEAT_INTERVAL: Duration = Duration::from_millis(90);

/// Estado da repetição de um único direcional. Apertar outro direcional troca o alvo; soltar
/// um direcional que não é o atual não interrompe o atual.
#[derive(Debug, Default)]
pub struct NavRepeater {
    held: Option<(NavCommand, Instant)>,
}

impl NavRepeater {
    pub fn new() -> Self {
        Self::default()
    }

    /// Um direcional foi pressionado em `now`. Retorna o comando a disparar imediatamente.
    pub fn press(&mut self, cmd: NavCommand, now: Instant) -> NavCommand {
        if cmd.direction().is_some() {
            self.held = Some((cmd, now + INITIAL_DELAY));
        }
        cmd
    }

    /// Um direcional foi solto.
    pub fn release(&mut self, cmd: NavCommand) {
        if self.held.is_some_and(|(held, _)| held == cmd) {
            self.held = None;
        }
    }

    /// Repetições devidas até `now` (no máximo uma por chamada, para não despejar uma rajada
    /// se a thread atrasar).
    pub fn tick(&mut self, now: Instant) -> Option<NavCommand> {
        let (cmd, due) = self.held?;
        if now < due {
            return None;
        }
        self.held = Some((cmd, now + REPEAT_INTERVAL));
        Some(cmd)
    }

    /// Quanto esperar até a próxima repetição, para a thread dormir exatamente isso.
    pub fn time_until_next(&self, now: Instant) -> Option<Duration> {
        self.held.map(|(_, due)| due.saturating_duration_since(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_immediately_then_after_delay_then_every_interval() {
        let t0 = Instant::now();
        let mut r = NavRepeater::new();
        assert_eq!(r.press(NavCommand::Down, t0), NavCommand::Down);
        assert_eq!(r.tick(t0 + Duration::from_millis(100)), None);
        assert_eq!(r.tick(t0 + INITIAL_DELAY), Some(NavCommand::Down));
        assert_eq!(r.tick(t0 + INITIAL_DELAY + Duration::from_millis(10)), None);
        assert_eq!(r.tick(t0 + INITIAL_DELAY + REPEAT_INTERVAL), Some(NavCommand::Down));
    }

    #[test]
    fn release_stops_and_other_release_does_not() {
        let t0 = Instant::now();
        let mut r = NavRepeater::new();
        r.press(NavCommand::Left, t0);
        r.release(NavCommand::Up);
        assert_eq!(r.tick(t0 + INITIAL_DELAY), Some(NavCommand::Left));
        r.release(NavCommand::Left);
        assert_eq!(r.tick(t0 + Duration::from_secs(5)), None);
        assert_eq!(r.time_until_next(t0), None);
    }

    #[test]
    fn new_direction_replaces_held_one_and_buttons_do_not_repeat() {
        let t0 = Instant::now();
        let mut r = NavRepeater::new();
        r.press(NavCommand::Left, t0);
        r.press(NavCommand::Right, t0 + Duration::from_millis(200));
        assert_eq!(r.tick(t0 + INITIAL_DELAY), None, "o atraso recomeça com o novo direcional");
        assert_eq!(
            r.tick(t0 + Duration::from_millis(200) + INITIAL_DELAY),
            Some(NavCommand::Right)
        );
        let mut r = NavRepeater::new();
        assert_eq!(r.press(NavCommand::Confirm, t0), NavCommand::Confirm);
        assert_eq!(r.tick(t0 + Duration::from_secs(5)), None);
    }
}
