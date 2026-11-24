//! Thread dedicada que lê controles via `gilrs` (XInput no Windows) e publica [`NavCommand`].

use std::sync::mpsc::{SendError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use gilrs::{Axis, Button, EventType, Gilrs};

use crate::NavCommand;
use crate::guide::GuidePoller;
use crate::repeat::NavRepeater;
use crate::stick::{StickEdge, StickState};

/// Espera máxima por um evento quando nada está sendo repetido. Curta porque, no Windows, o
/// botão Guide é lido por consulta a cada volta do laço (ver `guide`).
const IDLE_POLL: Duration = Duration::from_millis(40);

/// Mapeia um botão do layout padrão (posições Xbox) para um comando.
pub fn button_command(button: Button) -> Option<NavCommand> {
    Some(match button {
        Button::DPadUp => NavCommand::Up,
        Button::DPadDown => NavCommand::Down,
        Button::DPadLeft => NavCommand::Left,
        Button::DPadRight => NavCommand::Right,
        Button::South => NavCommand::Confirm,
        Button::East => NavCommand::Back,
        Button::Start => NavCommand::Menu,
        Button::Mode => NavCommand::Guide,
        _ => return None,
    })
}

/// Inicia a thread `bp-gamepad`. Ela termina quando o receptor do canal é descartado, ou logo
/// no início (com um aviso no log) se o backend de controles não puder ser aberto: o launcher
/// continua usável pelo teclado.
pub fn spawn(tx: Sender<NavCommand>) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("bp-gamepad".into())
        .spawn(move || {
            let mut gilrs = match Gilrs::new() {
                Ok(g) => g,
                Err(err) => {
                    tracing::warn!(error = %err, "controles indisponíveis; seguindo só com teclado");
                    return;
                }
            };
            for (_, pad) in gilrs.gamepads() {
                tracing::info!(name = pad.name(), "controle conectado");
            }
            if run(&mut gilrs, &tx).is_err() {
                tracing::debug!("receptor de NavCommand descartado; encerrando thread do controle");
            }
        })
}

fn run(gilrs: &mut Gilrs, tx: &Sender<NavCommand>) -> Result<(), SendError<NavCommand>> {
    let mut repeat = NavRepeater::new();
    let mut stick = StickState::new();
    let mut guide = GuidePoller::new();
    loop {
        if guide.poll() {
            tx.send(NavCommand::Guide)?;
        }
        let wait = repeat.time_until_next(Instant::now()).map_or(IDLE_POLL, |w| w.min(IDLE_POLL));
        if let Some(ev) = gilrs.next_event_blocking(Some(wait)) {
            let now = Instant::now();
            match ev.event {
                EventType::ButtonPressed(button, _) => {
                    if let Some(cmd) = button_command(button) {
                        tx.send(repeat.press(cmd, now))?;
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    if let Some(cmd) = button_command(button) {
                        repeat.release(cmd);
                    }
                }
                EventType::AxisChanged(axis, value, _) => {
                    let edges = match axis {
                        Axis::LeftStickX => stick.set_x(value),
                        Axis::LeftStickY => stick.set_y(value),
                        _ => Vec::new(),
                    };
                    for edge in edges {
                        match edge {
                            StickEdge::Pressed(cmd) => tx.send(repeat.press(cmd, now))?,
                            StickEdge::Released(cmd) => repeat.release(cmd),
                        }
                    }
                }
                EventType::Connected => {
                    tracing::info!(name = gilrs.gamepad(ev.id).name(), "controle conectado");
                }
                EventType::Disconnected => {
                    tracing::info!("controle desconectado");
                    repeat = NavRepeater::new();
                    stick = StickState::new();
                }
                _ => {}
            }
        }
        if let Some(cmd) = repeat.tick(Instant::now()) {
            tx.send(cmd)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_xbox_layout_buttons() {
        assert_eq!(button_command(Button::South), Some(NavCommand::Confirm));
        assert_eq!(button_command(Button::East), Some(NavCommand::Back));
        assert_eq!(button_command(Button::Start), Some(NavCommand::Menu));
        assert_eq!(button_command(Button::Mode), Some(NavCommand::Guide));
        assert_eq!(button_command(Button::DPadLeft), Some(NavCommand::Left));
        assert_eq!(button_command(Button::LeftTrigger), None);
    }
}
